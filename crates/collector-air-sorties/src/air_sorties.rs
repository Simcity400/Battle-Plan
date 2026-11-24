
use engine_allocation as allocation;
use engine_map_air::{
    map_air_carrier_operands, map_air_country_region_modifier, map_air_rocket_slots,
    map_air_sortie_inputs as native,
};
use engine_native_memory::{Memory, cpdx_array::Array, walk::{Buffer, Outcome}};
use engine_native::ccountry;
use engine_native_air::cair_regional_mission::WingIdentity;
use replay_data::{
    rows::AIR_MISSION_MAP_FLAGS, rows::AIR_MISSION_REACHED_IDS, rows::AirMissionInputRow,
    rows::AirSortieBaseCountryRow, rows::AirSortieModifierMetadataRow, rows::AirSortieModifierRow,
    rows::AirSortieSpecialBaseRow, rows::tables,
};
use replay_frame::frame::TableView;

use super::air_reference_witness::BaseWitness;
mod menu;
use crate::{
    table_capture::{grow_capacity, grow_vec},
    table_conversion::{table, unavailable},
};

pub trait RegionObserver {
    fn detection_overrides(
        &mut self,
        _emit: &mut dyn FnMut(u32, i64) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        Err("Air detection override writer certificate unavailable")
    }

    fn observed(
        &mut self,
        country: usize,
        region: usize,
        object: Option<map_air_country_region_modifier::Region>,
    );
}

impl RegionObserver for () {
    fn observed(&mut self, _: usize, _: usize, _: Option<map_air_country_region_modifier::Region>) {
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub missions: usize,
    pub reached: usize,
    pub rows: usize,
    pub keys: usize,
    pub hosts: usize,
}

collector_primitives::capacity_max!(Capacities {
    missions,
    reached,
    rows,
    keys,
    hosts
});

impl Capacities {
    pub fn presize(
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
    pointer: usize,
    pool: usize,
    air_base: u64,
    effective: u32,
    region: Option<u32>,
    membership: bool,
    operands: native::Mission,
    execution: Option<engine_native_air::cair_wing::fuel_inputs::Inputs>,
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

#[derive(Clone, Copy, Default)]
struct Key {
    scope: Scope,
    base: u64,
    slot: u32,
    region: u32,
    kind: i32,
    stateful: bool,
    fuel: bool,
    object: usize,
    country: usize,
}

impl Key {
    fn order(&self) -> (Scope, u64, u32, u32) {
        (self.scope, self.base, self.slot, self.region)
    }
}

#[derive(Default)]
struct Cached {
    weather_manager: Option<Result<usize, &'static str>>,
    weather: Option<Array>,
    containers: Option<(u64, Array)>,
    region_map: Option<(usize, map_air_country_region_modifier::CountryMap)>,
}

struct Rows<T> {
    rows: Vec<T>,
    len: usize,
    required: usize,
}

impl<T: Default> Rows<T> {
    fn new(capacity: usize) -> Self {
        Self {
            rows: allocation::owned_vec(allocation::default_vec(capacity)),
            len: 0,
            required: 0,
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
        Buffer::Growing(&mut self.rows).ensure(self.required)?;
        
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

pub struct Sorties {
    outcome: Outcome,
    metadata_outcome: Outcome,
    missions: Rows<Mission>,
    reached: Rows<u32>,
    containers: Rows<AirSortieBaseCountryRow>,
    modifiers: Rows<AirSortieModifierRow>,
    special: Rows<AirSortieSpecialBaseRow>,
    metadata: Option<AirSortieModifierMetadataRow>,
    keys: Rows<Key>,
    order: Vec<u32>,
    hosts: Vec<u32>,
    hosts_required: usize,
    hosts_short: usize,
}

const UNSET: Outcome = Outcome::Unavailable {
    reason: "air sortie inputs not captured",
};

impl Sorties {
    pub fn with_capacities(capacity: &Capacities) -> Self {
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

    pub fn reset(&mut self) {
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

    pub fn fail(&mut self, outcome: Outcome, metadata: Outcome) {
        self.reset();
        self.outcome = outcome;
        self.metadata_outcome = metadata;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.missions.bytes()
            + self.reached.bytes()
            + self.containers.bytes()
            + self.modifiers.bytes()
            + self.special.bytes()
            + self.keys.bytes()
            + (self.order.capacity() + self.hosts.capacity()) * size_of::<u32>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
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

    pub fn outcomes(&self) -> [Outcome; 5] {
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

    pub fn capture<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        metadata: bool,
        registered: impl Iterator<Item = Result<&'a WingIdentity, &'static str>>,
        witnesses: &[BaseWitness],
    ) {
        self.capture_mode(
            memory,
            base,
            game_state,
            metadata,
            registered,
            witnesses,
            &mut (),
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "The observer borrows the same capture inputs without bundling independent acquisition lifetimes."
    )]
    pub fn capture_observed<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        metadata: bool,
        registered: impl Iterator<Item = Result<&'a WingIdentity, &'static str>>,
        witnesses: &[BaseWitness],
        observer: &mut impl RegionObserver,
    ) {
        self.capture_mode(
            memory, base, game_state, metadata, registered, witnesses, observer,
        )
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the read operands travel beside the registered wings, base witnesses and region observer"
    )]
    fn capture_mode<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        metadata: bool,
        registered: impl Iterator<Item = Result<&'a WingIdentity, &'static str>>,
        witnesses: &[BaseWitness],
        observer: &mut impl RegionObserver,
    ) {
        self.reset();
        self.metadata_outcome = if metadata {
            UNSET
        } else {
            Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            }
        };
        self.outcome = match self.acquire(
            memory, base, game_state, metadata, registered, witnesses, observer,
        ) {
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

    #[expect(
        clippy::too_many_arguments,
        reason = "Keep the registered wing stream, witnesses and observer borrowed independently during acquisition."
    )]
    fn acquire<'a>(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        metadata: bool,
        registered: impl Iterator<Item = Result<&'a WingIdentity, &'static str>>,
        witnesses: &[BaseWitness],
        observer: &mut impl RegionObserver,
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
                execution: wing.fuel_inputs,
                ..Mission::default()
            })?;
        }
        self.read_missions(memory, base)?;
        Buffer::Growing(&mut self.order).ensure(self.missions.len)?;
        
        self.definitions(memory, base)?;
        self.queue_keys(memory, &countries, witnesses)?;
        let keys = &mut self.keys.rows[..self.keys.len];
        keys.sort_unstable_by_key(|key| (key.order(), !key.fuel));
        let mut cached = Cached::default();
        let mut previous = None;
        for index in 0..self.keys.len {
            let key = self.keys.rows[index];
            if previous == Some(key.order()) {
                continue;
            }
            previous = Some(key.order());
            self.operand(
                memory,
                base,
                game_state,
                &countries,
                &mut cached,
                key,
                observer,
            )?;
        }
        self.menu_modifiers(memory, base, game_state, &countries, &mut cached, observer)?;
        self.patrol_night(memory, base, game_state, &mut cached);
        self.overrides(memory, base)?;
        self.detection_overrides(observer)?;
        self.modifiers.rows[..self.modifiers.len]
            .sort_unstable_by_key(|row| (row.scope, row.subject, row.region));
        Ok(())
    }

    fn patrol_night(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        cached: &mut Cached,
    ) {
        let missions = &mut self.missions.rows[..self.missions.len];
        let mut count = 0;
        for (index, mission) in missions.iter().enumerate() {
            if native::patrol::demanded(&mission.operands) {
                self.order[count] = index as u32;
                count += 1;
            }
        }
        if count == 0 {
            return;
        }
        let manager = *cached
            .weather_manager
            .get_or_insert_with(|| native::weather_manager(memory, base, game_state));
        let Ok(Some(held)) = native::patrol::HeldNight::read(memory, game_state, manager) else {
            return;
        };
        let order = &mut self.order[..count];
        order.sort_unstable_by_key(|&index| missions[index as usize].operands.region_pointer);
        let mut previous = None;
        let mut applied = None;
        for &index in order.iter() {
            let mission = &mut missions[index as usize];
            let region = mission.operands.region_pointer;
            if previous != Some(region) {
                previous = Some(region);
                applied = held.applied(memory, base, region, mission.region).ok();
            }
            if let Some(normalization) = &mut mission.operands.normalization {
                normalization.patrol.night_applied = applied;
            }
        }
    }

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

    fn detection_overrides(
        &mut self,
        observer: &mut impl RegionObserver,
    ) -> Result<(), &'static str> {
        let first = self.modifiers.len;
        let result = observer.detection_overrides(&mut |slot, raw| {
            self.modifiers.push(AirSortieModifierRow {
                scope: AirSortieModifierRow::DETECTION_OVERRIDE,
                subject: slot,
                region: 0,
                raw,
            })
        });
        if result.is_err() {
            self.modifiers.len = first;
        }
        self.modifiers.push(AirSortieModifierRow {
            scope: AirSortieModifierRow::DETECTION_OVERRIDE_STATUS,
            subject: 0,
            region: 0,
            raw: i64::from(result.is_ok()),
        })?;
        Ok(())
    }

    fn read_missions(&mut self, memory: &impl Memory, base: usize) -> Result<(), &'static str> {
        let missions = &mut self.missions.rows[..self.missions.len];
        let mut spotting = native::patrol::SpottingFactors::default();
        for chunk in missions.chunks_mut(native::MISSION_BATCH) {
            let mut pointers = [0; native::MISSION_BATCH];
            let mut operands = [native::Mission::default(); native::MISSION_BATCH];
            for (pointer, mission) in pointers.iter_mut().zip(chunk.iter()) {
                *pointer = mission.pointer;
            }
            if chunk.iter().all(|mission| mission.execution.is_some()) {
                let mut execution = [engine_native_air::cair_wing::fuel_inputs::Inputs::default();
                    native::MISSION_BATCH];
                for (row, mission) in execution.iter_mut().zip(chunk.iter()) {
                    *row = mission
                        .execution
                        .expect("complete same-pass execution observations");
                }
                native::read_missions_with_spotting(
                    memory,
                    base,
                    &pointers[..chunk.len()],
                    Some(&execution[..chunk.len()]),
                    &mut spotting,
                    &mut operands[..chunk.len()],
                )?;
            } else {
                native::read_missions_with_spotting(
                    memory,
                    base,
                    &pointers[..chunk.len()],
                    None,
                    &mut spotting,
                    &mut operands[..chunk.len()],
                )?;
            }
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
                Buffer::Growing(&mut self.reached.rows).ensure(
                    start
                        .checked_add(ids)
                        .ok_or("sortie reached count overflow")?,
                )?;
                
                let Some(output) = self.reached.rows.get_mut(start..start + ids) else {
                    continue;
                };
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

    fn definitions(&mut self, memory: &impl Memory, base: usize) -> Result<(), &'static str> {
        let missions = &mut self.missions.rows[..self.missions.len];
        let order = &mut self.order[..missions.len()];
        for (index, slot) in order.iter_mut().enumerate() {
            *slot = index as u32;
        }
        order.sort_unstable_by_key(|&index| missions[index as usize].pool);
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
                    if region == 0 {
                        return Err("air mission region 0 has no supported weather record");
                    }
                    self.keys.push(Key {
                        scope: Scope::Weather,
                        region,
                        ..Key::default()
                    })?;
                }
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

    #[expect(
        clippy::too_many_arguments,
        reason = "The selected key shares the existing country reader and cache while forwarding its reached region."
    )]
    fn operand<M: Memory>(
        &mut self,
        memory: &M,
        base: usize,
        game_state: usize,
        countries: &ccountry::Reader<'_, M>,
        cached: &mut Cached,
        key: Key,
        observer: &mut impl RegionObserver,
    ) -> Result<(), &'static str> {
        let raw = match key.scope {
            Scope::Container => {
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
                let manager = *cached
                    .weather_manager
                    .get_or_insert_with(|| native::weather_manager(memory, base, game_state));
                let regions = match cached.weather {
                    Some(regions) => regions,
                    None => *cached
                        .weather
                        .insert(native::weather_regions_from_manager(memory, manager?)?),
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
                let object = map.region(memory, base, key.object)?;
                observer.observed(key.country, key.object, object);
                object
                    .map_or(Ok(Default::default()), |object| object.efficiency(memory))?
                    .raw
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

    pub fn tables(&self) -> [TableView; 5] {
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
                    interception_alert_gate: (mission
                        .operands
                        .interception_alert_gate
                        .map(u32::from))
                    .into(),
                    current_type: (mission.operands.current_type).into(),
                    effective_aircraft: (mission.operands.effective_aircraft).into(),
                    detection_stat_13: (mission
                        .operands
                        .normalization
                        .and_then(|input| input.detection[0]))
                    .into(),
                    detection_stat_14: (mission
                        .operands
                        .normalization
                        .and_then(|input| input.detection[1]))
                    .into(),
                    patrol_disruption_raw: (mission
                        .operands
                        .normalization
                        .and_then(|input| input.patrol.disruption))
                    .into(),
                    patrol_attack_raw: (mission
                        .operands
                        .normalization
                        .and_then(|input| input.patrol.attack))
                    .into(),
                    patrol_defence_raw: (mission
                        .operands
                        .normalization
                        .and_then(|input| input.patrol.defence))
                    .into(),
                    patrol_speed_raw: (mission
                        .operands
                        .normalization
                        .and_then(|input| input.patrol.speed))
                    .into(),
                    patrol_wing_attack_raw: (mission
                        .operands
                        .normalization
                        .and_then(|input| input.patrol.attack_modifier))
                    .into(),
                    patrol_wing_defence_raw: (mission
                        .operands
                        .normalization
                        .and_then(|input| input.patrol.defence_modifier))
                    .into(),
                    patrol_night_applied_raw: (mission
                        .operands
                        .normalization
                        .and_then(|input| input.patrol.night_applied))
                    .into(),
                    normalization: (mission.operands.normalization.map(|input| {
                        replay_data::rows::AirMissionNormalization {
                            night_stat: input.night_stat,
                            modifiers: input.modifiers.map(|entry| {
                                entry.map(|entry| replay_data::rows::AirNormalizationEntry {
                                    present: entry.present,
                                    raw: entry.raw,
                                })
                            }),
                        }
                    }))
                    .into(),
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
