//! Stored operands of the flying-aircraft map's count and admission.
//!
//! The game draws a count it recomputes on demand, so its stored
//! inputs are recorded for the registered missions only: each mission's own
//! operands, its base's containers for the wing's effective country and that
//! country's volunteer hosts, the regional and country modifiers, and the
//! host-base operands, and the console efficiency overrides. Everything the
//! stored-count alternative would replace lives here. Missions are read in gathered batches, each pool definition and
//! each country's volunteer hosts once, the weather manager once, and every
//! other operand once per sorted key.

use engine::{
    Memory, allocation,
    cair_regional_mission::WingIdentity,
    ccountry,
    cpdx_array::Array,
    map_air_carrier_operands, map_air_country_region_modifier, map_air_rocket_slots,
    map_air_sortie_inputs as native,
    walk::{Buffer, Outcome},
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

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub missions: usize,
    pub reached: usize,
    pub rows: usize,
    pub keys: usize,
    /// One country's volunteer-host vector.
    pub hosts: usize,
}

impl Capacities {
    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let [missions, reached, hosts, overrides] = native::storage_demand(memory, base, state)?;
        self.missions = self.missions.max(missions);
        self.reached = self.reached.max(reached);
        self.hosts = self.hosts.max(hosts);
        self.rows = self.rows.max(
            missions
                .checked_add(overrides)
                .ok_or("sortie sizing row count overflow")?,
        );
        self.keys = self.keys.max(missions);
        Ok(())
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

/// Fixed fixture storage or worker rows that grow before publication.
struct Rows<T> {
    rows: Vec<T>,
    len: usize,
    required: usize,
    growing: bool,
}

impl<T: Default> Rows<T> {
    fn new(capacity: usize) -> Self {
        Self {
            rows: allocation::owned_vec(allocation::default_vec(capacity)),
            len: 0,
            required: 0,
            growing: false,
        }
    }
    fn reset(&mut self) {
        self.len = 0;
        self.required = 0;
    }
    fn push(&mut self, row: T) -> Result<(), &'static str> {
        self.required = self
            .required
            .checked_add(1)
            .ok_or("sortie row count overflow")?;
        if self.growing {
            Buffer::Growing(&mut self.rows).ensure(self.required)?;
        }
        if let Some(slot) = self.rows.get_mut(self.len) {
            *slot = row;
            self.len += 1;
        }
        Ok(())
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
            order: allocation::owned_vec(allocation::default_vec(capacity.missions)),
            hosts: allocation::owned_vec(allocation::default_vec(capacity.hosts)),
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
    #[cfg(test)]
    pub(super) fn capture<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        metadata: bool,
        registered: impl Iterator<Item = Result<&'a WingIdentity, &'static str>>,
        witnesses: &[BaseWitness],
    ) {
        self.capture_mode(
            memory, base, game_state, metadata, registered, witnesses, false,
        )
    }

    pub(super) fn capture_growing<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        metadata: bool,
        registered: impl Iterator<Item = Result<&'a WingIdentity, &'static str>>,
        witnesses: &[BaseWitness],
    ) {
        self.capture_mode(
            memory, base, game_state, metadata, registered, witnesses, true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn capture_mode<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        metadata: bool,
        registered: impl Iterator<Item = Result<&'a WingIdentity, &'static str>>,
        witnesses: &[BaseWitness],
        growing: bool,
    ) {
        self.reset();
        self.missions.growing = growing;
        self.reached.growing = growing;
        self.containers.growing = growing;
        self.modifiers.growing = growing;
        self.special.growing = growing;
        self.keys.growing = growing;
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
            })?;
        }
        self.read_missions(memory, base)?;
        if self.missions.growing {
            Buffer::Growing(&mut self.order).ensure(self.missions.len)?;
        }
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
        native::efficiency_overrides_fallible(memory, base, |slot, raw| {
            self.modifiers.push(AirSortieModifierRow {
                scope: AirSortieModifierRow::EFFICIENCY_OVERRIDE,
                subject: slot,
                region: 0,
                raw,
            })?;
            Ok(())
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
                self.reached.required = self
                    .reached
                    .required
                    .checked_add(ids)
                    .ok_or("sortie reached count overflow")?;
                if self.reached.growing {
                    Buffer::Growing(&mut self.reached.rows).ensure(
                        start
                            .checked_add(ids)
                            .ok_or("sortie reached count overflow")?,
                    )?;
                }
                let Some(output) = self.reached.rows.get_mut(start..start + ids) else {
                    continue;
                };
                // The region's provinces are appended in roster order,
                // and the scenario draws index into that order.
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
        // The distinct pools in pool order, gathered up to the first a copy
        // or identity stopped at; that pool and the rest read as before.
        let mut start = 0;
        while start < order.len() {
            let mut pools = [0_usize; native::DEFINITION_BATCH];
            let mut count = 0;
            let mut end = start;
            while end < order.len() {
                let pool = missions[order[end] as usize].pool;
                if count == 0 || pools[count - 1] != pool {
                    if count == pools.len() {
                        break;
                    }
                    pools[count] = pool;
                    count += 1;
                }
                end += 1;
            }
            let mut flags = [0_u64; native::DEFINITION_BATCH];
            let read =
                native::definition_air_flags_gathered(memory, base, &pools[..count], &mut flags);
            for (flag, &pool) in flags[read..count].iter_mut().zip(&pools[read..count]) {
                *flag = native::definition_air_flags(memory, base, pool)?;
            }
            let mut pool = 0;
            for &index in &order[start..end] {
                let mission = &mut missions[index as usize];
                if mission.pool != pools[pool] {
                    pool += 1;
                }
                mission.definition_map_flags = flags[pool] & AIR_MISSION_MAP_FLAGS;
            }
            start = end;
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
            let count = if self.keys.growing {
                native::volunteer_hosts_growing(memory, country.pointer, &mut self.hosts)?
            } else {
                native::volunteer_hosts(memory, country.pointer, &mut self.hosts)?
            };
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
                self.keys.push(container(country.canonical_slot, true))?;
                for index in 0..hosts {
                    self.keys.push(container(self.hosts[index], false))?;
                }
                self.keys.push(Key {
                    scope: Scope::Base,
                    base: mission.air_base,
                    kind: pool_base.kind,
                    stateful: pool_base.state.is_some(),
                    object: pool_base.object,
                    ..Key::default()
                })?;
                let region = mission.region.unwrap_or(0);
                if mission.region.is_some() {
                    // Region zero would select unsupported weather record zero.
                    if region == 0 {
                        return Err("air mission region 0 has no supported weather record");
                    }
                    self.keys.push(Key {
                        scope: Scope::Weather,
                        region,
                        ..Key::default()
                    })?;
                }
                // Country terms apply only for a positive effective tag.
                if (effective as i32) > 0 {
                    self.keys.push(Key {
                        scope: Scope::Country,
                        slot: country.canonical_slot,
                        country: country.pointer,
                        ..Key::default()
                    })?;
                    if mission.operands.region_pointer != 0 {
                        self.keys.push(Key {
                            scope: Scope::CountryRegion,
                            slot: country.canonical_slot,
                            region,
                            object: mission.operands.region_pointer,
                            country: country.pointer,
                            ..Key::default()
                        })?;
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
                        self.containers.push(row)?;
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
                    self.special.push(row)?;
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
            })?;
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
                    reached_provinces: allocation::owned_vec(allocation::copy_vec(
                        &self.reached.rows
                            [mission.reached_start..mission.reached_start + mission.reached_len],
                    )),
                    definition_map_flags: mission.definition_map_flags,
                    cas_ratio_raw: mission.operands.cas_ratio_raw.into(),
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
mod tests;
