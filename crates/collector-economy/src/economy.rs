
use crate::{
    clock::qpc,
    projection::{Projection, project_table, scalar_table},
    raw::RawLog,
    read_accounting,
    table_capture::{Arena, CaptureRows, bounded, grow_vec},
    table_conversion::unavailable,
};
use engine_allocation::{copy_vec, filled, owned_vec};
use engine_events::ccountry_experience_status;
use engine_fuel::{ccountry_fuel_status, fuel_consumer_data, fuel_history_entry};
use engine_map_air::foreign_homebase_node;
use engine_native_memory::{Memory, cpdx_array::Array, msvc_string, walk::Outcome};
use engine_native::{ccountry, ccountry_supply_system, cgame_state};
use engine_native_parts::{hub_motorization_entry, supply_node_record};
use engine_production::{cpurchase_contract, market_contract_manager};
use engine_railways::{disrupted_supply_bucket, sprovince_supply_data};
use engine_resources::{ccountry_resources, cresource_exchange, cstrategic_resource};
use replay_data::{rows, rows::TableId, rows::tables};
use replay_frame::frame::TableInput;
use std::mem::size_of;

pub const N: usize = 12;
pub const TABLES: [TableId; N] = [
    TableId::CountryFuel,
    TableId::CountryExperience,
    TableId::ProvinceSupply,
    TableId::HubMotorization,
    TableId::DisruptedSupply,
    TableId::ForeignHomebases,
    TableId::ResourceDefinitions,
    TableId::CountryResources,
    TableId::ResourceExchanges,
    TableId::PurchaseContracts,
    TableId::CountryFuelConsumers,
    TableId::FuelDemandParts,
];

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub foreign_sources: usize,
    pub demand: super::fuel_demand::Capacities,
}

collector_primitives::capacity_max!(Capacities {
    rows,
    foreign_sources,
    demand
});

const FUEL_CONSUMERS: usize = ccountry_fuel_status::MAX_CONSUMERS as usize;

impl Capacities {
    /// # Safety
    /// Run only during attach or the active session's serial sizing pass against the authenticated
    /// session image, while market observer storage remains live and teardown is excluded.
    pub unsafe fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let rows = |outcome| match outcome {
            Outcome::Rows { len } => Ok(len),
            Outcome::Truncated { len, unread_rows } => len
                .checked_add(unread_rows as usize)
                .ok_or("economy sizing row count overflow"),
            Outcome::Unavailable { reason } => Err(reason),
        };
        let mut market_demand = 0_usize;
        let mut countries = 0_usize;
        let resources = ccountry_resources::storage_demand_with_country(
            memory,
            base,
            state,
            |_, resources| {
                market_demand = market_demand
                    .checked_add(collector_observers::market_changes::schedule_storage(
                        memory, base, resources,
                    )?)
                    .ok_or("market storage demand overflow")?;
                countries = countries
                    .checked_add(1)
                    .ok_or("market country demand overflow")?;
                Ok(())
            },
        )?;
        for (index, demand) in [6, 7, 8].into_iter().zip(resources) {
            crate::table_capture::grow_capacity(&mut self.rows[index], demand);
        }
        for (index, outcome) in [
            (2, sprovince_supply_data::count(memory, base, state)),
            (
                3,
                ccountry_supply_system::count(memory, base, state),
            ),
            (
                4,
                disrupted_supply_bucket::count(memory, base, state),
            ),
            (
                9,
                cpurchase_contract::count(memory, base, state),
            ),
        ] {
            crate::table_capture::grow_capacity(&mut self.rows[index], rows(outcome)?);
        }
        // SAFETY: Caller guarantees attach or serial sizing pass with live market observer storage.
        unsafe {
            collector_observers::market_changes::presize(
                market_demand
                    .checked_add(countries)
                    .ok_or("market storage demand overflow")?,
                countries.max(self.rows[9]),
            );
        }
        Ok(())
    }

    pub fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[0], rows(slots, 1));
        raise(&mut self.rows[1], rows(slots, 1));
        raise(&mut self.rows[10], rows(slots, FUEL_CONSUMERS));
        self.demand.raise_for_countries(slots);
    }
}

pub struct Captured {
    fuel: CaptureRows<ccountry_fuel_status::Row>,
    demand: super::fuel_demand::Captured,
    experience: CaptureRows<ccountry_experience_status::Row>,
    supply: CaptureRows<sprovince_supply_data::Row>,
    hubs: CaptureRows<ccountry_supply_system::Row>,
    disrupted: CaptureRows<disrupted_supply_bucket::Row>,
    foreign: CaptureRows<foreign_homebase_node::Row>,
    definitions: CaptureRows<cstrategic_resource::Row>,
    resources: CaptureRows<ccountry_resources::Row>,
    exchanges: CaptureRows<cresource_exchange::Row>,
    purchases: CaptureRows<cpurchase_contract::Row>,
    fuel_consumers: CaptureRows<ccountry_fuel_status::ConsumerRow>,
    sources: Arena<u8>,
    exchange_scratch: Vec<cresource_exchange::Membership>,
    purchase_scratch: Vec<cpurchase_contract::Membership>,
    pub rest_raw_mode: bool,
    resources_active: bool,
    other_active: bool,
    daily_released: bool,
    last_capacity: Capacities,
}

impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            foreign_sources: 0,
            demand: Default::default(),
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            fuel: CaptureRows::new(capacity.rows[0]),
            demand: super::fuel_demand::Captured::with_capacities(&capacity.demand),
            experience: CaptureRows::new(capacity.rows[1]),
            supply: CaptureRows::new(capacity.rows[2]),
            hubs: CaptureRows::new(capacity.rows[3]),
            disrupted: CaptureRows::new(capacity.rows[4]),
            foreign: CaptureRows::new(capacity.rows[5]),
            definitions: CaptureRows::new(capacity.rows[6]),
            resources: CaptureRows::new(capacity.rows[7]),
            exchanges: CaptureRows::new(capacity.rows[8]),
            purchases: CaptureRows::new(capacity.rows[9]),
            fuel_consumers: CaptureRows::new(capacity.rows[10]),
            sources: Arena::new(capacity.foreign_sources),
            exchange_scratch: owned_vec(filled(
                capacity.rows[8],
                cresource_exchange::Membership::default(),
            )),
            purchase_scratch: owned_vec(filled(
                capacity.rows[9],
                cpurchase_contract::Membership::default(),
            )),
            rest_raw_mode: false,
            resources_active: false,
            other_active: false,
            daily_released: false,
            last_capacity: *capacity,
        }
    }

    pub fn reset(&mut self) {
        self.fuel.reset();
        self.demand.reset();
        self.experience.reset();
        self.supply.reset();
        self.hubs.reset();
        self.disrupted.reset();
        self.foreign.reset();
        self.definitions.reset();
        self.resources.reset();
        self.exchanges.reset();
        self.purchases.reset();
        self.fuel_consumers.reset();
        self.sources.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.fuel.allocated_bytes()
            + self.demand.allocated_bytes()
            + self.experience.allocated_bytes()
            + self.supply.allocated_bytes()
            + self.hubs.allocated_bytes()
            + self.disrupted.allocated_bytes()
            + self.foreign.allocated_bytes()
            + self.definitions.allocated_bytes()
            + self.resources.allocated_bytes()
            + self.exchanges.allocated_bytes()
            + self.purchases.allocated_bytes()
            + self.fuel_consumers.allocated_bytes()
            + self.sources.allocated_bytes()
            + self.exchange_scratch.capacity() * size_of::<cresource_exchange::Membership>()
            + self.purchase_scratch.capacity() * size_of::<cpurchase_contract::Membership>()
    }

    pub fn release_daily(&mut self) {
        self.last_capacity.rows[0] = self.last_capacity.rows[0].max(self.fuel.rows.len());
        self.last_capacity.rows[1] = self.last_capacity.rows[1].max(self.experience.rows.len());
        self.last_capacity.rows[3] = self.last_capacity.rows[3].max(self.hubs.rows.len());
        self.last_capacity.rows[4] = self.last_capacity.rows[4].max(self.disrupted.rows.len());
        self.last_capacity.rows[5] = self.last_capacity.rows[5].max(self.foreign.rows.len());
        self.last_capacity.rows[9] = self.last_capacity.rows[9].max(self.purchases.rows.len());
        self.last_capacity.rows[10] = self
            .last_capacity
            .rows[10]
            .max(self.fuel_consumers.rows.len());
        self.last_capacity.foreign_sources = self
            .last_capacity
            .foreign_sources
            .max(self.sources.first.len());
        self.demand.release_daily();
        self.fuel.release();
        self.experience.release();
        self.hubs.release();
        self.disrupted.release();
        self.foreign.release();
        self.purchases.release();
        self.fuel_consumers.release();
        self.sources.release();
        self.purchase_scratch = Vec::new();
        self.daily_released = true;
    }

    pub(super) fn rehydrate_daily(&mut self) {
        if !self.daily_released {
            return;
        }
        self.daily_released = false;
        self.demand.rehydrate_daily();
        grow_vec(&mut self.fuel.rows, self.last_capacity.rows[0]);
        grow_vec(&mut self.experience.rows, self.last_capacity.rows[1]);
        grow_vec(&mut self.hubs.rows, self.last_capacity.rows[3]);
        grow_vec(&mut self.disrupted.rows, self.last_capacity.rows[4]);
        grow_vec(&mut self.foreign.rows, self.last_capacity.rows[5]);
        grow_vec(&mut self.purchases.rows, self.last_capacity.rows[9]);
        grow_vec(&mut self.fuel_consumers.rows, self.last_capacity.rows[10]);
        grow_vec(&mut self.sources.first, self.last_capacity.foreign_sources);
        grow_vec(&mut self.purchase_scratch, self.last_capacity.rows[9]);
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.demand.grow(&mut capacity.demand);
        self.supply.grow(&mut capacity.rows[2]);
        self.definitions.grow(&mut capacity.rows[6]);
        self.resources.grow(&mut capacity.rows[7]);
        self.exchanges.grow(&mut capacity.rows[8]);
        grow_vec(&mut self.exchange_scratch, capacity.rows[8]);
        if self.daily_released {
            self.fuel.record_demand(&mut capacity.rows[0]);
            self.experience.record_demand(&mut capacity.rows[1]);
            self.hubs.record_demand(&mut capacity.rows[3]);
            self.disrupted.record_demand(&mut capacity.rows[4]);
            self.foreign.record_demand(&mut capacity.rows[5]);
            self.purchases.record_demand(&mut capacity.rows[9]);
            self.fuel_consumers.record_demand(&mut capacity.rows[10]);
            self.sources.record_demand(&mut capacity.foreign_sources);
        } else {
            self.fuel.grow(&mut capacity.rows[0]);
            self.experience.grow(&mut capacity.rows[1]);
            self.hubs.grow(&mut capacity.rows[3]);
            self.disrupted.grow(&mut capacity.rows[4]);
            self.foreign.grow(&mut capacity.rows[5]);
            self.purchases.grow(&mut capacity.rows[9]);
            self.fuel_consumers.grow(&mut capacity.rows[10]);
            self.sources.grow(&mut capacity.foreign_sources);
            grow_vec(&mut self.purchase_scratch, capacity.rows[9]);
        }
        self.last_capacity = *capacity;
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_buffers(memory, base, game_state, hour, force_full)
    }

    pub fn fail_final_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.demand.fail_capture(hour, reason);
        if !self.rest_raw_mode {
            self.set_schedule(hour, force_full);
            self.fail_economy_rest_capture(reason);
        }
    }

    fn capture_buffers(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.reset();
        self.set_schedule(hour, force_full);
        let mut ticks = [0; N];
        if self.other_active {
            self.rehydrate_daily();
            let _reads = read_accounting::table(TABLES[0]);
            let begin = qpc();
            self.demand.capture(memory, base, game_state, hour);
            
            ticks[0] = elapsed(begin);
        }
        if self.rest_raw_mode {
            return ticks;
        }
        macro_rules! plain {
            ($index:literal, $field:ident, $fixed:path, $worker:path) => {{
                let _reads = read_accounting::table(TABLES[$index]);
                let begin = qpc();
                capture_rows(&mut self.$field, |rows| {
                    $worker(memory, base, game_state, rows)
                    
                });
                ticks[$index] = elapsed(begin);
            }};
        }
        if self.other_active {
            let _reads = read_accounting::table(TABLES[0]);
            let begin = qpc();
            self.capture_fuel_buffers(memory, base, game_state);
            ticks[0] = ticks[0].saturating_add(elapsed(begin));
            plain!(
                1,
                experience,
                ccountry_experience_status::walk,
                ccountry_experience_status::walk
            );
        }
        plain!(
            2,
            supply,
            sprovince_supply_data::walk,
            sprovince_supply_data::walk
        );
        if self.other_active {
            plain!(
                3,
                hubs,
                ccountry_supply_system::walk,
                ccountry_supply_system::walk
            );
            plain!(
                4,
                disrupted,
                disrupted_supply_bucket::walk,
                disrupted_supply_bucket::walk
            );
            let _reads = read_accounting::table(TABLES[5]);
            let begin = qpc();
            let acquired = foreign_homebase_node::walk_buffer(
                memory,
                base,
                game_state,
                engine_native_memory::walk::Buffer::Growing(&mut self.foreign.rows),
                engine_native_memory::walk::Buffer::Growing(&mut self.sources.first),
            );
            self.foreign.outcome = bounded(acquired.outcome, self.foreign.rows.len());
            self.sources.len = acquired.bytes_len;
            self.sources.required = acquired.required_bytes;
            if acquired.required_bytes < acquired.bytes_len
                || !valid_sources(
                    self.foreign.outcome,
                    &self.foreign.rows,
                    &self.sources.first,
                    acquired.bytes_len,
                )
            {
                self.foreign.outcome = invalid_sources();
            }
            ticks[5] = elapsed(begin);
        }
        if self.resources_active {
            let mut reads = read_accounting::table(TABLES[6]);
            let begin = qpc();
            capture_rows(&mut self.definitions, |rows| {
                cstrategic_resource::walk_buffer(
                    memory,
                    base,
                    engine_native_memory::walk::Buffer::Growing(rows),
                )
            });
            ticks[6] = elapsed(begin);
            let descriptors = match self.definitions.outcome {
                Outcome::Rows { len } => self.definitions.rows.get(..len),
                _ => None,
            };
            reads.select(TABLES[7]);
            let begin = qpc();
            if let Some(descriptors) = descriptors {
                capture_rows(&mut self.resources, |rows| {
                    ccountry_resources::walk_buffer(
                        memory,
                        base,
                        game_state,
                        descriptors,
                        engine_native_memory::walk::Buffer::Growing(rows),
                    )
                });
            } else {
                self.resources.outcome = unavailable_descriptors();
            }
            ticks[7] = elapsed(begin);
            reads.select(TABLES[8]);
            let begin = qpc();
            if let Some(descriptors) = descriptors {
                self.exchanges.outcome = bounded(
                    cresource_exchange::walk_buffer(
                        memory,
                        base,
                        game_state,
                        descriptors,
                        engine_native_memory::walk::Buffer::Growing(&mut self.exchanges.rows),
                        engine_native_memory::walk::Buffer::Growing(&mut self.exchange_scratch),
                    ),
                    self.exchanges.rows.len(),
                );
            } else {
                self.exchanges.outcome = unavailable_descriptors();
            }
            ticks[8] = elapsed(begin);
        }
        if self.other_active {
            let _reads = read_accounting::table(TABLES[9]);
            let begin = qpc();
            capture_rows(&mut self.purchases, |rows| {
                cpurchase_contract::walk_buffer(
                    memory,
                    base,
                    game_state,
                    engine_native_memory::walk::Buffer::Growing(rows),
                    engine_native_memory::walk::Buffer::Growing(&mut self.purchase_scratch),
                )
            });
            ticks[9] = elapsed(begin);
        }
        ticks
    }

    fn capture_fuel_buffers(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
    ) {
        let outcomes = ccountry_fuel_status::walk_buffer(
            memory,
            base,
            game_state,
            engine_native_memory::walk::Buffer::Growing(&mut self.fuel.rows),
            engine_native_memory::walk::Buffer::Growing(&mut self.fuel_consumers.rows),
        );
        self.fuel.outcome = bounded(outcomes.fuel, self.fuel.rows.len());
        self.fuel_consumers.outcome = bounded(outcomes.consumers, self.fuel_consumers.rows.len());
    }

    fn set_schedule(&mut self, hour: i32, force_full: bool) {
        self.resources_active =
            replay_data::cadence::should_walk(TableId::CountryResources, hour, force_full);
        debug_assert_eq!(
            self.resources_active,
            replay_data::cadence::should_walk(TableId::ResourceExchanges, hour, force_full)
        );
        debug_assert_eq!(
            self.resources_active,
            replay_data::cadence::should_walk(TableId::ResourceDefinitions, hour, force_full)
        );
        self.other_active =
            replay_data::cadence::should_walk(TableId::CountryFuel, hour, force_full);
        self.mark_inactive();
    }

    fn mark_inactive(&mut self) {
        let skipped = skipped();
        if !self.resources_active {
            self.definitions.outcome = skipped;
            self.resources.outcome = skipped;
            self.exchanges.outcome = skipped;
        }
        if !self.other_active {
            self.fuel.outcome = skipped;
            self.experience.outcome = skipped;
            self.hubs.outcome = skipped;
            self.disrupted.outcome = skipped;
            self.foreign.outcome = skipped;
            self.purchases.outcome = skipped;
            self.fuel_consumers.outcome = skipped;
            self.sources.reset();
        }
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.fuel.outcome,
            self.experience.outcome,
            self.supply.outcome,
            self.hubs.outcome,
            self.disrupted.outcome,
            self.foreign.outcome,
            self.definitions.outcome,
            self.resources.outcome,
            self.exchanges.outcome,
            self.purchases.outcome,
            self.fuel_consumers.outcome,
            self.fuel.outcome,
        ]
    }

    pub fn table_inputs(&self, mut projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
        let outcomes = self.outcomes();
        let mut tables = [
            scalar_table::<tables::CountryFuel, _>(
                None,
                outcomes[0],
                &self.fuel.rows,
                |r| r.country as u128,
                |r| rows::CountryFuelRow {
                    country: r.country,
                    current_raw: r.current_raw,
                    maximum_raw: r.maximum_raw,
                    lend_lease_gain_raw: r.lend_lease_gain_raw,
                    ring_gain_raw: r.ring_gain_raw,
                    ring_consumed_raw: r.ring_consumed_raw,
                    ring_other_raw: r.ring_other_raw,
                    ring_received_0_raw: r.ring_received_raw[0],
                    ring_received_1_raw: r.ring_received_raw[1],
                    ring_received_2_raw: r.ring_received_raw[2],
                    awards: rows::Recorded::Value(rows::Retired),
                    demand: rows::Recorded::NotRecorded,
                    award_day: rows::Recorded::NotRecorded,
                    country_instance: rows::Recorded::Value(r.country_instance),
                    ..Default::default()
                },
            ),
            scalar_table::<tables::CountryExperience, _>(
                projection.as_deref_mut(),
                outcomes[1],
                &self.experience.rows,
                |r| r.country as u128,
                |r| rows::CountryExperienceRow {
                    country: r.country,
                    army_raw: r.army_raw,
                    army_daily_raw: r.army_daily_raw,
                    army_daily_training_raw: r.army_daily_training_raw,
                    navy_raw: r.navy_raw,
                    navy_daily_raw: r.navy_daily_raw,
                    navy_daily_training_raw: r.navy_daily_training_raw,
                    air_raw: r.air_raw,
                    air_daily_raw: r.air_daily_raw,
                    armies_for_training_raw: r.armies_for_training_raw,
                },
            ),
            scalar_table::<tables::ProvinceSupply, _>(
                projection.as_deref_mut(),
                outcomes[2],
                &self.supply.rows,
                |r| r.province as u128,
                |r| rows::ProvinceSupplyRow {
                    province: r.province,
                    controller_index: r.controller_index,
                    capital_country_index: r.capital_country_index,
                    supply_node_level: r.supply_node_level,
                    naval_base_level: r.naval_base_level,
                    flags: r.flags,
                },
            ),
            scalar_table::<tables::HubMotorization, _>(
                projection.as_deref_mut(),
                outcomes[3],
                &self.hubs.rows,
                |r| {
                    (r.owner as u128) << 96
                        | (r.province as u32 as u128) << 64
                        | (r.node_type as u32 as u128) << 32
                        | r.actor as u128
                },
                |r| rows::HubMotorizationRow {
                    owner: r.owner,
                    actor: r.actor,
                    node_index: r.node_index,
                    map_entry_index: r.map_entry_index,
                    province: r.province,
                    node_type: r.node_type,
                    level: r.level,
                },
            ),
            scalar_table::<tables::DisruptedSupply, _>(
                projection.as_deref_mut(),
                outcomes[4],
                &self.disrupted.rows,
                |r| (r.country as u128) << 64 | (r.province as u128) << 32 | r.node_type as u128,
                |r| rows::DisruptedSupplyRow {
                    country: r.country,
                    province: r.province,
                    node_type: r.node_type,
                    value: r.value,
                },
            ),
            self.foreign_table(projection.as_deref_mut(), outcomes[5]),
            scalar_table::<tables::ResourceDefinitions, _>(
                projection.as_deref_mut(),
                outcomes[6],
                &self.definitions.rows,
                |r| r.id as u128,
                |r| rows::ResourceDefinitionRow {
                    id: r.id,
                    token: r.token,
                    active: r.active,
                    factory_cost: r.factory_cost,
                },
            ),
            scalar_table::<tables::CountryResources, _>(
                projection.as_deref_mut(),
                outcomes[7],
                &self.resources.rows,
                |r| (r.country as u128) << 32 | r.resource as u128,
                |r| rows::CountryResourceRow {
                    country: r.country,
                    resource: r.resource,
                    produced: r.produced,
                    adjustment: r.adjustment,
                    imported: r.imported,
                    used: r.used,
                    special_total: r.special_total,
                    production_demand: r.production_demand,
                    to_export: r.to_export,
                    base_export: r.base_export,
                    exported: r.exported,
                },
            ),
            scalar_table::<tables::ResourceExchanges, _>(
                projection.as_deref_mut(),
                outcomes[8],
                &self.exchanges.rows,
                |r| r.id as u128,
                |r| rows::ResourceExchangeRow {
                    id: r.id,
                    exporter: r.exporter,
                    importer: r.importer,
                    resource: r.resource,
                    import_ordinal: r.import_ordinal,
                    export_ordinal: r.export_ordinal,
                    convoys_assigned: r.convoys_assigned,
                    convoys_required: r.convoys_required,
                    efficiency_a: r.efficiency_a,
                    efficiency_b: r.efficiency_b,
                    delivered: r.delivered,
                    required_factories: r.required_factories,
                    assigned_factories: r.assigned_factories,
                    no_trade_influence: r.no_trade_influence,
                    no_resources: r.no_resources,
                },
            ),
            scalar_table::<tables::PurchaseContracts, _>(
                projection.as_deref_mut(),
                outcomes[9],
                &self.purchases.rows,
                |r| r.id as u128,
                |r| rows::PurchaseContractRow {
                    id: r.id,
                    seller: r.seller,
                    buyer: r.buyer,
                    speed: r.speed,
                    convoys_assigned: r.convoys_assigned,
                    convoys_total: r.convoys_total,
                    route_efficiency_sum_raw: r.route_efficiency_sum_raw,
                    route_days: r.route_days,
                    factory_cic_progress_raw: r.factory_cic_progress_raw,
                    equipment_transfer_progress_raw: r.equipment_transfer_progress_raw,
                    collected_raw: r.collected_raw,
                    completed_deliveries: r.completed_deliveries,
                },
            ),
            scalar_table::<tables::CountryFuelConsumers, _>(
                projection,
                outcomes[10],
                &self.fuel_consumers.rows,
                |r| (r.country as u128) << 32 | r.index as u128,
                |r| rows::CountryFuelConsumerRow {
                    country: r.country,
                    index: r.index,
                    priority: r.priority,
                    requested_raw: r.requested_raw,
                    received_raw: r.received_raw,
                },
            ),
            unavailable("fuel demand parts were not split").into(),
        ];
        match super::fuel_demand::attach(&mut tables[0], &self.demand) {
            Ok(parts) => tables[11] = parts.into(),
            Err(error) => {
                let _ = engine_allocation::owned_vec::<()>(Err(error));
            }
        }
        tables
    }

    #[cfg(test)]
    pub fn tables(&self) -> [replay_frame::frame::TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection disabled"),
        })
    }

    fn foreign_table(
        &self,
        projection: Option<&mut Projection<'_>>,
        outcome: Outcome,
    ) -> TableInput {
        if self.sources.required < self.sources.len
            || !valid_sources(
                outcome,
                &self.foreign.rows,
                &self.sources.first,
                self.sources.len,
            )
        {
            return unavailable("foreign homebase source range exceeds captured arena").into();
        }
        project_table::<tables::ForeignHomebases, _>(
            projection,
            outcome,
            &self.foreign.rows,
            |r| (r.country as u128) << 32 | r.ordinal as u128,
            |r, previous| {
                r.country == previous.country
                    && r.ordinal == previous.ordinal
                    && r.province == previous.province
                    && r.supply == previous.supply
                    && r.start == previous.start
                    && r.penalty == previous.penalty
                    && r.added_penalty == previous.added_penalty
                    && r.duration == previous.duration
                    && r.hours == previous.hours
                    && r.decay == previous.decay
                    && r.base == previous.base
                    && self.sources.first[r.source_start..r.source_start + r.source_len]
                        == previous.source
            },
            |r| rows::ForeignHomebaseRow {
                country: r.country,
                ordinal: r.ordinal,
                province: r.province,
                supply: r.supply,
                start: r.start,
                penalty: r.penalty,
                added_penalty: r.added_penalty,
                duration: r.duration,
                hours: r.hours,
                decay: r.decay,
                base: r.base,
                source: owned_vec(copy_vec(
                    &self.sources.first[r.source_start..r.source_start + r.source_len],
                )),
            },
        )
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn capture_rows<R>(captured: &mut CaptureRows<R>, mut walk: impl FnMut(&mut Vec<R>) -> Outcome) {
    captured.outcome = bounded(walk(&mut captured.rows), captured.rows.len());
}

fn unavailable_descriptors() -> Outcome {
    Outcome::Unavailable {
        reason: "resource descriptor census incomplete or inconsistent",
    }
}

fn invalid_sources() -> Outcome {
    Outcome::Unavailable {
        reason: "foreign homebase source range exceeds captured arena",
    }
}

fn valid_sources(
    outcome: Outcome,
    rows: &[foreign_homebase_node::Row],
    bytes: &[u8],
    len: usize,
) -> bool {
    len <= bytes.len()
        && rows.get(..outcome.len()).is_some_and(|rows| {
            rows.iter().all(|r| {
                r.source_start
                    .checked_add(r.source_len)
                    .is_some_and(|end| end <= len)
            })
        })
}

mod owned;

#[cfg(test)]
use owned::OwnedResources;
use owned::skipped;
pub use owned::{
    EconomyRestCopyContext, EconomyRestCopyReport, EconomyRestJob,
    copy_economy_rest_refresh,
};

#[cfg(test)]
mod tests;
