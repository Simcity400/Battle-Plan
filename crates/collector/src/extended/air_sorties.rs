//! Stored operands of the flying-aircraft map's count and admission.
//!
//! The game draws a count it recomputes on demand (140F63330), so its stored
//! inputs are recorded for the registered missions only: each mission's own
//! operands, its base's containers for the wing's effective country and that
//! country's volunteer hosts, the regional and country modifiers, and the
//! host-base operands, and the console efficiency overrides. Everything the
//! stored-count alternative would replace lives here. Missions are read in gathered batches, each pool definition and
//! each country's volunteer hosts once, the weather manager once, and every
//! other operand once per sorted key.

use engine::{
    Memory, cair_regional_mission::WingIdentity, ccountry, cpdx_array::Array,
    map_air_carrier_operands, map_air_country_region_modifier, map_air_rocket_slots,
    map_air_sortie_inputs as native, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{
        AIR_MISSION_MAP_FLAGS, AIR_MISSION_REACHED_IDS, AirMissionInputRow,
        AirSortieBaseCountryRow, AirSortieModifierMetadataRow, AirSortieModifierRow,
        AirSortieSpecialBaseRow, tables,
    },
};

use super::air_reference_witness::BaseWitness;
use crate::{
    spooling::{table, unavailable},
    table_capture::{grow_capacity, grow_vec},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub missions: usize,
    pub reached: usize,
    pub rows: usize,
    pub keys: usize,
    /// One country's volunteer-host vector.
    pub hosts: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            missions: 4096,
            reached: 65_536,
            rows: 2048,
            keys: 8192,
            hosts: 256,
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Mission {
    wing: u64,
    /// The embedded CAirMission and the wing's pool, acquisition context.
    pointer: usize,
    pool: usize,
    air_base: u64,
    effective: u32,
    region: Option<u32>,
    membership: bool,
    operands: native::Mission,
    definition_map_flags: u64,
    reached_start: usize,
    reached_len: usize,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    #[default]
    Container,
    Base,
    Weather,
    Country,
    CountryRegion,
}

/// One operand to read, with the acquisition pointers it needs.
#[derive(Clone, Copy, Default)]
struct Key {
    scope: Scope,
    base: u64,
    slot: u32,
    region: u32,
    kind: i32,
    stateful: bool,
    /// A container key for an effective country, whose fuel the law reads.
    fuel: bool,
    object: usize,
    country: usize,
}

impl Key {
    fn order(&self) -> (Scope, u64, u32, u32) {
        (self.scope, self.base, self.slot, self.region)
    }
}

/// Headers read once per hour (weather), per base (containers) or per
/// country (its region map); keys arrive grouped by base and country.
#[derive(Default)]
struct Cached {
    weather: Option<Array>,
    containers: Option<(u64, Array)>,
    region_map: Option<(usize, map_air_country_region_modifier::CountryMap)>,
}

/// A preallocated buffer that counts its demand past capacity.
struct Rows<T> {
    rows: Vec<T>,
    len: usize,
    required: usize,
}

impl<T: Default + Clone> Rows<T> {
    fn new(capacity: usize) -> Self {
        Self {
            rows: vec![T::default(); capacity],
            len: 0,
            required: 0,
        }
    }
    fn reset(&mut self) {
        self.len = 0;
        self.required = 0;
    }
    fn push(&mut self, row: T) {
        self.required += 1;
        if let Some(slot) = self.rows.get_mut(self.len) {
            *slot = row;
            self.len += 1;
        }
    }
    fn short(&self) -> usize {
        self.required - self.len
    }
    fn grow(&mut self, capacity: &mut usize) {
        grow_capacity(capacity, self.required);
        grow_vec(&mut self.rows, *capacity);
    }
    fn bytes(&self) -> usize {
        self.rows.capacity() * size_of::<T>()
    }
}

pub(super) struct Sorties {
    outcome: Outcome,
    /// The Static metadata's own outcome when it was not read.
    metadata_outcome: Outcome,
    missions: Rows<Mission>,
    reached: Rows<u32>,
    containers: Rows<AirSortieBaseCountryRow>,
    modifiers: Rows<AirSortieModifierRow>,
    special: Rows<AirSortieSpecialBaseRow>,
    metadata: Option<AirSortieModifierMetadataRow>,
    keys: Rows<Key>,
    /// Mission indices, sorted by pool and then by effective country.
    order: Vec<u32>,
    /// One country's volunteer hosts, then their slots; `hosts_required` is
    /// the longest vector seen and `hosts_short` the countries it cut off.
    hosts: Vec<u32>,
    hosts_required: usize,
    hosts_short: usize,
}

const UNSET: Outcome = Outcome::Unavailable {
    reason: "air sortie inputs not captured",
};

impl Sorties {
    pub(super) fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            outcome: UNSET,
            metadata_outcome: UNSET,
            missions: Rows::new(capacity.missions),
            reached: Rows::new(capacity.reached),
            containers: Rows::new(capacity.rows),
            modifiers: Rows::new(capacity.rows),
            special: Rows::new(capacity.rows),
            metadata: None,
            keys: Rows::new(capacity.keys),
            order: vec![0; capacity.missions],
            hosts: vec![0; capacity.hosts],
            hosts_required: 0,
            hosts_short: 0,
        }
    }

    pub(super) fn reset(&mut self) {
        self.outcome = UNSET;
        self.metadata_outcome = UNSET;
        self.missions.reset();
        self.reached.reset();
        self.containers.reset();
        self.modifiers.reset();
        self.special.reset();
        self.metadata = None;
        self.keys.reset();
        self.hosts_required = 0;
        self.hosts_short = 0;
    }

    /// `metadata` is the Static metadata's outcome, which is off-slot on most hours.
    pub(super) fn fail(&mut self, outcome: Outcome, metadata: Outcome) {
        self.reset();
        self.outcome = outcome;
        self.metadata_outcome = metadata;
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        self.missions.bytes()
            + self.reached.bytes()
            + self.containers.bytes()
            + self.modifiers.bytes()
            + self.special.bytes()
            + self.keys.bytes()
            + (self.order.capacity() + self.hosts.capacity()) * size_of::<u32>()
    }

    pub(super) fn grow(&mut self, capacity: &mut Capacities) {
        self.missions.grow(&mut capacity.missions);
        grow_vec(&mut self.order, capacity.missions);
        self.reached.grow(&mut capacity.reached);
        for required in [
            self.containers.required,
            self.modifiers.required,
            self.special.required,
        ] {
            grow_capacity(&mut capacity.rows, required);
        }
        grow_vec(&mut self.containers.rows, capacity.rows);
        grow_vec(&mut self.modifiers.rows, capacity.rows);
        grow_vec(&mut self.special.rows, capacity.rows);
        self.keys.grow(&mut capacity.keys);
        grow_capacity(&mut capacity.hosts, self.hosts_required);
        grow_vec(&mut self.hosts, capacity.hosts);
    }

    /// The four hourly tables share one outcome; metadata is Static.
    pub(super) fn outcomes(&self) -> [Outcome; 5] {
        let metadata = match &self.metadata {
            Some(_) => Outcome::Rows { len: 1 },
            None => self.metadata_outcome,
        };
        [
            self.outcome,
            self.outcome,
            self.outcome,
            self.outcome,
            metadata,
        ]
    }

    /// `registered` are the registered wings, `witnesses` the probed bases
    /// sorted by handle. Any unreadable operand fails every sortie table.
    pub(super) fn capture<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        metadata: bool,
        registered: impl Iterator<Item = Result<&'a WingIdentity, &'static str>>,
        witnesses: &[BaseWitness],
    ) {
        self.reset();
        self.metadata_outcome = if metadata {
            UNSET
        } else {
            Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            }
        };
        self.outcome = match self.acquire(memory, base, game_state, metadata, registered, witnesses)
        {
            Ok(()) => {
                let short = self.missions.short()
                    + self.reached.short()
                    + self.containers.short()
                    + self.modifiers.short()
                    + self.special.short()
                    + self.keys.short()
                    + self.hosts_short;
                if short == 0 {
                    Outcome::Rows { len: 0 }
                } else {
                    Outcome::Truncated {
                        len: 0,
                        unread_rows: short.min(u32::MAX as usize) as u32,
                    }
                }
            }
            Err(reason) => {
                if metadata {
                    self.metadata = None;
                    self.metadata_outcome = Outcome::Unavailable { reason };
                }
                Outcome::Unavailable { reason }
            }
        };
    }

    fn acquire<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        metadata: bool,
        registered: impl Iterator<Item = Result<&'a WingIdentity, &'static str>>,
        witnesses: &[BaseWitness],
    ) -> Result<(), &'static str> {
        if metadata {
            let row = native::read_modifier_metadata(memory, base)?;
            self.metadata = Some(AirSortieModifierMetadataRow {
                flags: row.flags,
                feature_selector: row.feature_selector,
                callback_kind: row.callback_kind,
            });
        }
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        for wing in registered {
            let wing = wing?;
            self.missions.push(Mission {
                wing: wing.wing,
                pointer: wing.mission_pointer,
                pool: wing.pool,
                air_base: wing.air_base,
                effective: wing.effective_country(),
                region: wing.region,
                membership: wing.mission_mask & AIR_MISSION_REACHED_IDS != 0,
                ..Mission::default()
            });
        }
        self.read_missions(memory, base)?;
        self.definitions(memory, base)?;
        self.queue_keys(memory, &countries, witnesses)?;
        let keys = &mut self.keys.rows[..self.keys.len];
        // A fuel key sorts first, so a deduplicated container keeps its fuel.
        keys.sort_unstable_by_key(|key| (key.order(), !key.fuel));
        let mut cached = Cached::default();
        let mut previous = None;
        for index in 0..self.keys.len {
            let key = self.keys.rows[index];
            if previous == Some(key.order()) {
                continue;
            }
            previous = Some(key.order());
            self.operand(memory, base, game_state, &countries, &mut cached, key)?;
        }
        self.overrides(memory, base)
    }

    /// Every console override, in slot order after the keyed modifiers. The
    /// filler keeps one entry per slot, so a repeat is refused.
    fn overrides(&mut self, memory: &impl Memory, base: usize) -> Result<(), &'static str> {
        let first = self.modifiers.len;
        native::efficiency_overrides(memory, base, |slot, raw| {
            self.modifiers.push(AirSortieModifierRow {
                scope: AirSortieModifierRow::EFFICIENCY_OVERRIDE,
                subject: slot,
                region: 0,
                raw,
            });
        })?;
        let overrides = &mut self.modifiers.rows[first..self.modifiers.len];
        overrides.sort_unstable_by_key(|row| row.subject);
        if overrides
            .windows(2)
            .any(|pair| pair[0].subject == pair[1].subject)
        {
            return Err("sortie efficiency overrides repeat a country");
        }
        Ok(())
    }

    /// Mission operands in gathered batches, then the reached IDs. A mission
    /// whose IDs do not fit is dropped and counted short.
    fn read_missions(&mut self, memory: &impl Memory, base: usize) -> Result<(), &'static str> {
        let missions = &mut self.missions.rows[..self.missions.len];
        for chunk in missions.chunks_mut(native::MISSION_BATCH) {
            let mut pointers = [0; native::MISSION_BATCH];
            let mut operands = [native::Mission::default(); native::MISSION_BATCH];
            for (pointer, mission) in pointers.iter_mut().zip(chunk.iter()) {
                *pointer = mission.pointer;
            }
            native::read_missions(
                memory,
                base,
                &pointers[..chunk.len()],
                &mut operands[..chunk.len()],
            )?;
            for (mission, operands) in chunk.iter_mut().zip(operands) {
                mission.operands = operands;
            }
        }
        let mut kept = 0;
        for index in 0..self.missions.len {
            let mut mission = self.missions.rows[index];
            if mission.membership {
                let ids = mission.operands.reached.count as usize;
                let start = self.reached.len;
                self.reached.required += ids;
                let Some(output) = self.reached.rows.get_mut(start..start + ids) else {
                    continue;
                };
                // in roster order, and the scenario draws index into it.
                native::read_reached(memory, mission.operands.reached, output)?;
                self.reached.len += ids;
                (mission.reached_start, mission.reached_len) = (start, ids);
            }
            self.missions.rows[kept] = mission;
            kept += 1;
        }
        self.missions.len = kept;
        Ok(())
    }

    /// Each distinct pool's definition flags, read once.
    fn definitions(&mut self, memory: &impl Memory, base: usize) -> Result<(), &'static str> {
        let missions = &mut self.missions.rows[..self.missions.len];
        let order = &mut self.order[..missions.len()];
        for (index, slot) in order.iter_mut().enumerate() {
            *slot = index as u32;
        }
        order.sort_unstable_by_key(|&index| missions[index as usize].pool);
        let mut previous = None;
        let mut flags = 0;
        for &index in order.iter() {
            let mission = &mut missions[index as usize];
            if previous != Some(mission.pool) {
                previous = Some(mission.pool);
                flags = native::definition_air_flags(memory, base, mission.pool)?
                    & AIR_MISSION_MAP_FLAGS;
            }
            mission.definition_map_flags = flags;
        }
        Ok(())
    }

    /// Queue every mission's operand keys, resolving each effective country
    /// and reading its volunteer hosts once. A mission whose pool base does
    /// not resolve flies nowhere and needs none; the viewer reports it.
    fn queue_keys<M: Memory>(
        &mut self,
        memory: &M,
        countries: &ccountry::Reader<'_, M>,
        witnesses: &[BaseWitness],
    ) -> Result<(), &'static str> {
        let len = self.missions.len;
        let missions = &self.missions.rows[..len];
        let order = &mut self.order[..len];
        order.sort_unstable_by_key(|&index| missions[index as usize].effective);
        let pool_base = |mission: &Mission| {
            witnesses
                .binary_search_by_key(&mission.air_base, |witness| witness.handle)
                .ok()
                .and_then(|index| witnesses[index].base)
        };
        let mut first = 0;
        while first < len {
            let effective = missions[self.order[first] as usize].effective;
            let end = first
                + self.order[first..len]
                    .iter()
                    .take_while(|&&index| missions[index as usize].effective == effective)
                    .count();
            let members = first..end;
            first = end;
            if !self.order[members.clone()]
                .iter()
                .any(|&index| pool_base(&missions[index as usize]).is_some())
            {
                continue;
            }
            let country = countries.resolve(effective)?;
            let count = native::volunteer_hosts(memory, country.pointer, &mut self.hosts)?;
            self.hosts_required = self.hosts_required.max(count);
            let mut hosts = 0;
            if count > self.hosts.len() {
                self.hosts_short += 1;
            } else {
                for index in 0..count {
                    // A zero or unlinked host names no country and so no container.
                    let host = self.hosts[index];
                    if host != 0
                        && let Some(slot) = countries.link(host)?
                    {
                        self.hosts[hosts] = slot;
                        hosts += 1;
                    }
                }
            }
            for position in members {
                let mission = missions[self.order[position] as usize];
                let Some(pool_base) = pool_base(&mission) else {
                    continue;
                };
                let container = |slot, fuel| Key {
                    scope: Scope::Container,
                    base: mission.air_base,
                    slot,
                    fuel,
                    object: pool_base.object,
                    ..Key::default()
                };
                self.keys.push(container(country.canonical_slot, true));
                for index in 0..hosts {
                    self.keys.push(container(self.hosts[index], false));
                }
                self.keys.push(Key {
                    scope: Scope::Base,
                    base: mission.air_base,
                    kind: pool_base.kind,
                    stateful: pool_base.state.is_some(),
                    object: pool_base.object,
                    ..Key::default()
                });
                let region = mission.region.unwrap_or(0);
                if mission.region.is_some() {
                    // 140F04E40 would index weather record 0, which is unsupported.
                    if region == 0 {
                        return Err("air mission region 0 has no supported weather record");
                    }
                    self.keys.push(Key {
                        scope: Scope::Weather,
                        region,
                        ..Key::default()
                    });
                }
                // 140F63380 adds the country terms only for a positive effective tag.
                if (effective as i32) > 0 {
                    self.keys.push(Key {
                        scope: Scope::Country,
                        slot: country.canonical_slot,
                        country: country.pointer,
                        ..Key::default()
                    });
                    if mission.operands.region_pointer != 0 {
                        self.keys.push(Key {
                            scope: Scope::CountryRegion,
                            slot: country.canonical_slot,
                            region,
                            object: mission.operands.region_pointer,
                            country: country.pointer,
                            ..Key::default()
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn operand<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game_state: usize,
        countries: &ccountry::Reader<'_, M>,
        cached: &mut Cached,
        key: Key,
    ) -> Result<(), &'static str> {
        let raw = match key.scope {
            Scope::Container => {
                // Container keys arrive grouped by base.
                let containers = match cached.containers {
                    Some((air_base, containers)) if air_base == key.base => containers,
                    _ => {
                        let containers = native::country_containers(memory, key.object)?;
                        cached.containers = Some((key.base, containers));
                        containers
                    }
                };
                if let Some(container) =
                    native::read_container(memory, base, containers, key.slot, countries)?
                {
                    let fuel = |raw| key.fuel.then_some(raw);
                    let row = AirSortieBaseCountryRow {
                        base: key.base,
                        country: key.slot,
                        capacity_raw: container.capacity_raw,
                        supply_raw: container.supply_raw,
                        fuel_demand_raw: fuel(container.fuel_demand_raw),
                        fuel_allocated_raw: fuel(container.fuel_allocated_raw),
                    };
                    let zero = |raw: Option<i64>| raw.unwrap_or(0) == 0;
                    if row.capacity_raw != 0
                        || row.supply_raw != 0
                        || !zero(row.fuel_demand_raw)
                        || !zero(row.fuel_allocated_raw)
                    {
                        self.containers.push(row);
                    }
                }
                return Ok(());
            }
            Scope::Base => {
                let mut row = AirSortieSpecialBaseRow {
                    base: key.base,
                    ..Default::default()
                };
                if key.kind == 1 && key.stateful {
                    row.rocket_slots =
                        map_air_rocket_slots::state_rocket_slots(memory, base, key.object)?
                            .map(i32::from);
                } else if !key.stateful
                    && key.kind != 2
                    // A stale host omits only its wings; the viewer names them.
                    && let Some(host) =
                        map_air_carrier_operands::read(memory, base, key.object, key.base)?
                {
                    row.carrier_stance = host.carrier_stance;
                    row.leader_modifier_raw = host.leader_modifier.raw;
                    row.task_force_combats = host.task_force_combats;
                    row.task_force_region = host.task_force_region;
                }
                if row.rocket_slots.is_some()
                    || row.carrier_stance.is_some()
                    || row.leader_modifier_raw != 0
                    || row.task_force_combats.is_some()
                {
                    self.special.push(row);
                }
                return Ok(());
            }
            Scope::Weather => {
                let regions = match cached.weather {
                    Some(regions) => regions,
                    None => *cached
                        .weather
                        .insert(native::weather_regions(memory, base, game_state)?),
                };
                native::weather_modifier(memory, base, regions, key.region)?.raw
            }
            Scope::Country => native::country_modifier(memory, key.country)?.raw,
            Scope::CountryRegion => {
                let map = match cached.region_map {
                    Some((country, map)) if country == key.country => map,
                    _ => {
                        let map = map_air_country_region_modifier::country_map(
                            memory,
                            base,
                            key.country,
                        )?;
                        cached.region_map = Some((key.country, map));
                        map
                    }
                };
                map.lookup(memory, base, key.object)?.raw
            }
        };
        if raw != 0 {
            let scope = match key.scope {
                Scope::Weather => AirSortieModifierRow::WEATHER,
                Scope::Country => AirSortieModifierRow::COUNTRY,
                _ => AirSortieModifierRow::COUNTRY_REGION,
            };
            self.modifiers.push(AirSortieModifierRow {
                scope,
                subject: key.slot,
                region: key.region,
                raw,
            });
        }
        Ok(())
    }

    /// Mission inputs, base countries, modifiers, special bases and metadata.
    pub(super) fn tables(&self) -> [TableView; 5] {
        let metadata = match (&self.metadata, self.metadata_outcome) {
            (Some(row), _) => table::<tables::AirSortieModifierMetadata, _>(
                Outcome::Rows { len: 1 },
                std::slice::from_ref(row),
                Clone::clone,
            ),
            (None, outcome) => unavailable(match outcome {
                Outcome::Unavailable { reason } => reason,
                _ => "air sortie metadata not captured",
            }),
        };
        let outcome = match self.outcome {
            Outcome::Unavailable { reason } => {
                return [
                    unavailable(reason),
                    unavailable(reason),
                    unavailable(reason),
                    unavailable(reason),
                    metadata,
                ];
            }
            outcome => outcome,
        };
        let with = |len: usize| match outcome {
            Outcome::Truncated { unread_rows, .. } => Outcome::Truncated { len, unread_rows },
            _ => Outcome::Rows { len },
        };
        let missions = &self.missions.rows[..self.missions.len];
        [
            table::<tables::AirMissionInputs, _>(with(missions.len()), missions, |mission| {
                AirMissionInputRow {
                    wing: mission.wing,
                    coverage_raw: mission.operands.coverage_raw,
                    reassignment_penalty_raw: mission.operands.reassignment_penalty_raw,
                    last_target_province: mission.operands.last_target_province,
                    reached_count: mission.operands.reached.count as i32,
                    reached_provinces: self.reached.rows
                        [mission.reached_start..mission.reached_start + mission.reached_len]
                        .to_vec(),
                    definition_map_flags: mission.definition_map_flags,
                }
            }),
            table::<tables::AirSortieBaseCountries, _>(
                with(self.containers.len),
                &self.containers.rows[..self.containers.len],
                Clone::clone,
            ),
            table::<tables::AirSortieModifiers, _>(
                with(self.modifiers.len),
                &self.modifiers.rows[..self.modifiers.len],
                Clone::clone,
            ),
            table::<tables::AirSortieSpecialBases, _>(
                with(self.special.len),
                &self.special.rows[..self.special.len],
                Clone::clone,
            ),
            metadata,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::{
        air_mission, cair_base, cair_wing_pool, ccountry_air_container, cgame_state,
        cstrategic_region, csub_unit_definition, cweather_manager,
        map_air_base_reference::BaseReference, memory::SliceMemory, sweather_per_region,
    };
    use std::cell::Cell;

    const MODULE: usize = 0x4000_0000;
    const START: usize = 0x10_0000;
    const COUNTRY_SIZE: usize = 0x1500;
    /// `map_air_country_region_modifier`'s country map at CCountry+0x4D8.
    const COUNTRY_REGION_MAP: usize = 0x4d8;
    const REGION_COVERAGE: usize = 0x50;
    const REASSIGNMENT_PENALTY: usize = 0xc0;
    const REACHED: u32 = 16;
    /// `map_air_sortie_inputs`'s console override vector.
    const EFFICIENCY_OVERRIDES_RVA: usize = 0x0331_7208;

    /// A flat address space; every vtable is `MODULE + rva`.
    struct World {
        bytes: Vec<u8>,
    }

    impl World {
        fn alloc(&mut self, size: usize) -> usize {
            let at = (START + self.bytes.len() + 0xf) & !0xf;
            self.bytes.resize(at + size - START, 0);
            at
        }
        fn put(&mut self, at: usize, value: &[u8]) {
            self.bytes[at - START..at - START + value.len()].copy_from_slice(value);
        }
        fn q(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn d(&mut self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }
        fn vtable(&mut self, at: usize, rva: usize) {
            self.q(at, (MODULE + rva) as u64);
        }
        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.q(at, data as u64);
            self.d(at + 8, count);
            self.d(at + 12, count);
        }
        /// One sorted key-0x11 accumulator entry.
        fn accumulator(&mut self, at: usize, raw: i64) {
            let entries = self.alloc(16);
            self.d(entries, native::SORTIE_MODIFIER_KEY);
            self.q(entries + 8, raw as u64);
            self.array(at, entries, 1);
        }
    }

    /// The world, and the module's empty console override vector.
    struct Route<'a> {
        world: SliceMemory<'a>,
    }

    const NO_OVERRIDES: [u8; 16] = [0; 16];

    impl Memory for Route<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address >= MODULE {
                SliceMemory {
                    base: MODULE + EFFICIENCY_OVERRIDES_RVA,
                    bytes: &NO_OVERRIDES,
                }
                .copy(address, destination)
            } else {
                self.world.copy(address, destination)
            }
        }
    }

    /// Counts guarded calls as `LiveMemory` issues them: one per copy, per
    /// scattered batch and per record gather.
    struct Counting<'a> {
        inner: Route<'a>,
        reads: Cell<usize>,
    }

    impl<'a> Counting<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Self {
                inner: Route {
                    world: SliceMemory { base: START, bytes },
                },
                reads: Cell::new(0),
            }
        }
        fn count(&self) {
            self.reads.set(self.reads.get() + 1);
        }
    }

    impl Memory for Counting<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.count();
            self.inner.copy(address, destination)
        }
        fn copy_scattered<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.count();
            self.inner.copy_scattered(addresses, destinations)
        }
        fn copy_scattered_prefetched<const N: usize>(
            &self,
            addresses: [usize; N],
            destinations: &mut [&mut [u8]; N],
        ) -> usize {
            self.count();
            self.inner
                .copy_scattered_prefetched(addresses, destinations)
        }
        fn copy_records(
            &self,
            sources: engine::memory::RecordSources<'_>,
            layout: &engine::memory::RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.count();
            self.inner.copy_records(sources, layout, destination)
        }
    }

    struct Fixture {
        world: World,
        game_state: usize,
        wings: Vec<WingIdentity>,
        witnesses: Vec<BaseWitness>,
    }

    /// `missions` registered missions over `countries` countries, of which
    /// 300 fly with two volunteer hosts each, 170 regions, a base per four
    /// missions and a pool per eight, each mission reaching 16 provinces.
    fn fixture(missions: usize, countries: u32) -> Fixture {
        const FLYING: u32 = 300;
        const REGIONS: u32 = 170;
        let mut world = World { bytes: Vec::new() };
        let game_state = world.alloc(0x1000);
        let slots = countries + 1;
        let table = world.alloc(slots as usize * 8);
        let links = world.alloc(slots as usize * 4);
        world.array(game_state + cgame_state::COUNTRIES, table, slots);
        world.array(game_state + cgame_state::COUNTRY_LINKS, links, slots);
        let containers = world.alloc(slots as usize * 8);
        for slot in 1..slots {
            let country = world.alloc(COUNTRY_SIZE);
            world.q(table + slot as usize * 8, country as u64);
            world.d(links + slot as usize * 4, slot);
            world.vtable(country, ccountry::VTABLE_RVA);
            world.d(country + ccountry::ID, slot);
            world.accumulator(country + ccountry::MODIFIER_ACCUMULATOR, 2_000);
            // An empty country-region map: sentinel links to itself.
            let map = country + COUNTRY_REGION_MAP;
            let sentinel = world.alloc(16);
            let buckets = world.alloc(16);
            world.q(sentinel, sentinel as u64);
            world.q(sentinel + 8, sentinel as u64);
            world.q(buckets, sentinel as u64);
            world.q(buckets + 8, sentinel as u64);
            world.q(map + 0x08, sentinel as u64);
            world.q(map + 0x18, buckets as u64);
            world.q(map + 0x38, 1);
            if slot <= FLYING {
                let hosts = world.alloc(8);
                world.d(hosts, (slot % countries) + 1);
                world.d(hosts + 4, ((slot + 7) % countries) + 1);
                world.array(country + ccountry::VOLUNTEER_HOSTS, hosts, 2);
                let container = world.alloc(ccountry_air_container::SIZE);
                world.vtable(container, ccountry_air_container::VTABLE_RVA);
                world.d(container + ccountry_air_container::COUNTRY, slot);
                for (offset, raw) in [
                    (ccountry_air_container::SUPPLY_RATIO_RAW, 90_000),
                    (ccountry_air_container::SORTIE_CAPACITY_FACTOR_RAW, 80_000),
                    (ccountry_air_container::FUEL_DEMAND_RAW, 1_000),
                    (ccountry_air_container::FUEL_ALLOCATED_RAW, 700),
                ] {
                    world.q(container + offset, raw);
                }
                world.q(containers + slot as usize * 8, container as u64);
            }
        }
        let manager = world.alloc(0x100);
        world.q(game_state + cgame_state::WEATHER_MANAGER, manager as u64);
        world.vtable(manager, cweather_manager::VTABLE_RVA);
        let records = world.alloc((REGIONS as usize + 1) * sweather_per_region::STRIDE);
        world.array(manager + cweather_manager::REGIONS, records, REGIONS + 1);
        let mut regions = vec![0];
        for id in 1..=REGIONS {
            let record = records + id as usize * sweather_per_region::STRIDE;
            world.vtable(record, sweather_per_region::VTABLE_RVA);
            world.d(record + sweather_per_region::ID, id);
            world.accumulator(record + sweather_per_region::MODIFIERS, -5_000);
            let region = world.alloc(0x100);
            world.vtable(region, cstrategic_region::VTABLE_RVA);
            world.vtable(
                region + cstrategic_region::SECONDARY,
                cstrategic_region::SECONDARY_VTABLE_RVA,
            );
            regions.push(region);
        }
        let definitions: Vec<usize> = (0..20)
            .map(|_| {
                let definition = world.alloc(csub_unit_definition::AIR_FLAGS + 8);
                world.vtable(definition, csub_unit_definition::VTABLE_RVA);
                definition
            })
            .collect();
        let pools: Vec<usize> = (0..missions.div_ceil(8))
            .map(|index| {
                let pool = world.alloc(cair_wing_pool::SIZE);
                let definition = definitions[index % definitions.len()];
                world.q(pool + cair_wing_pool::DEFINITION, definition as u64);
                pool
            })
            .collect();
        let mut witnesses: Vec<BaseWitness> = (0..missions.div_ceil(4))
            .map(|index| {
                let object = world.alloc(0x80);
                world.array(object + cair_base::COUNTRY_CONTAINERS, containers, slots);
                BaseWitness {
                    handle: ((index as u64 + 1) << 32) | u64::from(cair_base::TYPE),
                    base: Some(BaseReference {
                        object,
                        state: Some(index as u32 + 1),
                        province: Some(index as u32 + 1),
                        host: 0,
                        kind: 0,
                    }),
                    host_owner: None,
                    host_without_task_force: false,
                }
            })
            .collect();
        witnesses.sort_unstable_by_key(|witness| witness.handle);
        let wings = (0..missions)
            .map(|index| {
                let mission = world.alloc(0xd0);
                let region = index as u32 % REGIONS + 1;
                world.vtable(mission, air_mission::VTABLE_RVA);
                world.q(
                    mission + air_mission::REGION,
                    regions[region as usize] as u64,
                );
                let reached = world.alloc(REACHED as usize * 16);
                // Descending, so a sorted capture would show.
                for entry in 0..REACHED {
                    world.d(reached + entry as usize * 16, 1_000 + (REACHED - entry) * 3);
                }
                world.array(
                    mission + air_mission::REACHED_PROVINCES_DATA,
                    reached,
                    REACHED,
                );
                world.q(mission + REGION_COVERAGE, 90_000);
                world.q(mission + REASSIGNMENT_PENALTY, 10_000);
                world.d(mission + air_mission::LAST_TARGET_PROVINCE, 1_000);
                WingIdentity {
                    mission_pointer: mission,
                    pool: pools[index / 8],
                    wing: ((index as u64 + 1) << 32) | 54,
                    owner: index as u32 % FLYING + 1,
                    region: Some(region),
                    mission_mask: 1 << 4,
                    air_base: witnesses[index / 4].handle,
                    base_override: 0,
                    country_override: 0,
                }
            })
            .collect();
        Fixture {
            world,
            game_state,
            wings,
            witnesses,
        }
    }

    fn capacities() -> Capacities {
        Capacities {
            missions: 8192,
            reached: 1 << 17,
            rows: 16_384,
            keys: 65_536,
            ..Capacities::default()
        }
    }

    #[test]
    fn volunteer_hosts_past_capacity_truncate_until_grown() {
        let mut capacity = Capacities {
            hosts: 1,
            ..capacities()
        };
        let mut sorties = Sorties::with_capacities(&capacity);
        let fixture = fixture(8, 1_500);
        let memory = Counting::new(&fixture.world.bytes);
        let capture = |sorties: &mut Sorties| {
            sorties.capture(
                &memory,
                MODULE,
                fixture.game_state,
                false,
                fixture.wings.iter().map(Ok),
                &fixture.witnesses,
            )
        };
        // Two hosts do not fit: only the wings' own containers are read.
        capture(&mut sorties);
        assert!(matches!(sorties.outcome, Outcome::Truncated { .. }));
        assert_eq!(sorties.containers.len, 8);
        sorties.grow(&mut capacity);
        assert_eq!(capacity.hosts, 2);
        capture(&mut sorties);
        assert_eq!(sorties.outcome, Outcome::Rows { len: 0 });
        // Two bases, each with four owners and their hosts: nine slots apiece.
        assert_eq!(sorties.containers.len, 18);
        // Only the eight effective countries' rows carry fuel.
        let rows = &sorties.containers.rows[..18];
        assert_eq!(
            rows.iter()
                .filter(|row| row.fuel_demand_raw.is_some())
                .count(),
            8
        );
    }

    #[test]
    fn reached_provinces_keep_the_native_order() {
        let mut sorties = Sorties::with_capacities(&capacities());
        let fixture = fixture(2, 16);
        let memory = Counting::new(&fixture.world.bytes);
        sorties.capture(
            &memory,
            MODULE,
            fixture.game_state,
            false,
            fixture.wings.iter().map(Ok),
            &fixture.witnesses,
        );
        let mission = sorties.missions.rows[0];
        let reached = &sorties.reached.rows
            [mission.reached_start..mission.reached_start + mission.reached_len];
        let native: Vec<u32> = (0..REACHED)
            .map(|entry| 1_000 + (REACHED - entry) * 3)
            .collect();
        assert_eq!(reached, native);
    }

    /// Prints guarded calls per hour and the best of five capture times;
    /// run with `--release --nocapture` to measure.
    #[test]
    fn sortie_cost_scales_with_missions_not_countries() {
        let mut sorties = Sorties::with_capacities(&capacities());
        for (missions, countries) in [
            (1_000, 1_500),
            (2_000, 1_500),
            (5_000, 1_500),
            (5_000, 5_000),
        ] {
            let fixture = fixture(missions, countries);
            let memory = Counting::new(&fixture.world.bytes);
            let mut best = std::time::Duration::MAX;
            for _ in 0..5 {
                memory.reads.set(0);
                let begin = std::time::Instant::now();
                sorties.capture(
                    &memory,
                    MODULE,
                    fixture.game_state,
                    false,
                    fixture.wings.iter().map(Ok),
                    &fixture.witnesses,
                );
                best = best.min(begin.elapsed());
            }
            assert_eq!(sorties.outcome, Outcome::Rows { len: 0 });
            assert_eq!(sorties.missions.len, missions);
            let calls = memory.reads.get();
            println!(
                "{missions} missions, {countries} countries: {calls} guarded calls ({:.1}/mission), {best:?}",
                calls as f64 / missions as f64,
            );
            assert!(calls <= 30 * missions, "{calls} guarded calls");
        }
    }
}
