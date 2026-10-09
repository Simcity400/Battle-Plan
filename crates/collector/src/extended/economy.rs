//! Economy acquisition buffers. Growth and replay conversion run off-thread.

use crate::{
    projection::{Projection, project_table, scalar_table},
    raw::RawLog,
    read_accounting,
    sampling::qpc,
    spooling::unavailable,
    table_capture::{Arena, CaptureRows, bounded, grow_vec},
};
use engine::{
    Memory,
    allocation::{copy_vec, filled, owned_vec},
    ccountry, ccountry_experience_status, ccountry_fuel_status, ccountry_resources,
    ccountry_supply_system, cgame_state,
    cpdx_array::Array,
    cpurchase_contract, cresource_exchange, cstrategic_resource, disrupted_supply_bucket,
    foreign_homebase_node, fuel_consumer_data, fuel_history_entry, hub_motorization_entry,
    market_contract_manager, msvc_string, sprovince_supply_data, supply_node_record,
    walk::Outcome,
};
use replay::{
    frame::TableInput,
    rows::{self, TableId, tables},
};
use std::mem::size_of;

pub const N: usize = 11;
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
];

/// Definitions are authenticated once and reused by the two dependent walks.
/// Their read time is charged to ResourceDefinitions, not charged again to each
/// consumer. CountryResources and ResourceExchanges are hourly and the
/// definitions daily (replay::cadence): the census is walked every hour for
/// its two consumers and published only in its slot. The consumers' own
/// timings include their additional native reads.
/// One fuel walk reads each country's status once for both its tables and is
/// charged to CountryFuel.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("resource dictionary and consumers", 6, &[6, 7, 8]),
    ("fuel status and consumers", 0, &[0, 10]),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub foreign_sources: usize,
    pub demand: super::fuel_demand::Capacities,
}

/// Consumer rows per country, fixed by the engine's fuel status.
const FUEL_CONSUMERS: usize = ccountry_fuel_status::MAX_CONSUMERS as usize;

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [
                512,
                512,
                16_384,
                1024,
                256,
                512,
                16,
                4096,
                1024,
                128,
                512 * FUEL_CONSUMERS,
            ],
            foreign_sources: 16_384,
            demand: Default::default(),
        }
    }
}
impl Capacities {
    /// CountryFuel and CountryExperience hold one row per canonical country,
    /// and CountryFuelConsumers the engine's fixed number per country; CountryResources one per country and resource definition (the
    /// default's eight per country covers the vanilla roster; more grows).
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[0], rows(slots, 1));
        raise(&mut self.rows[1], rows(slots, 1));
        raise(&mut self.rows[7], rows(slots, 8));
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
    /// Raw-copy split gate for economy's hourly and scheduled tables.
    /// Set when an acquisition job returns a valid raw payload; `capture`
    /// then skips the typed walks and the decode worker fills the rows before
    /// the ready handoff. Preserved across `reset` (set per hour, cleared by
    /// the worker after decode).
    pub(crate) rest_raw_mode: bool,
    /// CountryResources, ResourceExchanges and the resource census they need
    /// are walked.
    resources_active: bool,
    /// ResourceDefinitions is in its daily slot.
    definitions_slot: bool,
    other_active: bool,
}

impl Captured {
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
            definitions_slot: false,
            other_active: false,
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

    pub fn grow(&mut self, capacity: &mut Capacities) {
        self.fuel.grow(&mut capacity.rows[0]);
        self.demand.grow(&mut capacity.demand);
        self.experience.grow(&mut capacity.rows[1]);
        self.supply.grow(&mut capacity.rows[2]);
        self.hubs.grow(&mut capacity.rows[3]);
        self.disrupted.grow(&mut capacity.rows[4]);
        self.foreign.grow(&mut capacity.rows[5]);
        self.definitions.grow(&mut capacity.rows[6]);
        self.resources.grow(&mut capacity.rows[7]);
        self.exchanges.grow(&mut capacity.rows[8]);
        self.purchases.grow(&mut capacity.rows[9]);
        self.fuel_consumers.grow(&mut capacity.rows[10]);
        self.sources.grow(&mut capacity.foreign_sources);
        grow_vec(&mut self.exchange_scratch, capacity.rows[8]);
        grow_vec(&mut self.purchase_scratch, capacity.rows[9]);
    }

    /// Bounded acquisition without allocation, encoding or I/O; returns QPC ticks.
    pub fn capture(
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
            let _reads = read_accounting::table(TABLES[0]);
            let begin = qpc();
            self.demand.capture(memory, base, game_state, hour);
            ticks[0] = elapsed(begin);
        }
        // A staged payload is installed by the decode worker.
        if self.rest_raw_mode {
            return ticks;
        }
        macro_rules! plain {
            ($index:literal, $field:ident, $walker:path) => {{
                let _reads = read_accounting::table(TABLES[$index]);
                let begin = qpc();
                capture_rows(&mut self.$field, |rows| {
                    $walker(memory, base, game_state, rows)
                });
                ticks[$index] = elapsed(begin);
            }};
        }
        if self.other_active {
            let _reads = read_accounting::table(TABLES[0]);
            let begin = qpc();
            self.capture_fuel(memory, base, game_state);
            ticks[0] = ticks[0].saturating_add(elapsed(begin));
            plain!(1, experience, ccountry_experience_status::walk);
        }
        plain!(2, supply, sprovince_supply_data::walk);
        if self.other_active {
            plain!(3, hubs, ccountry_supply_system::walk);
            plain!(4, disrupted, disrupted_supply_bucket::walk);
            let _reads = read_accounting::table(TABLES[5]);
            let begin = qpc();
            let acquired = foreign_homebase_node::walk(
                memory,
                base,
                game_state,
                &mut self.foreign.rows,
                &mut self.sources.first,
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
                cstrategic_resource::walk(memory, base, rows)
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
                    ccountry_resources::walk(memory, base, game_state, descriptors, rows)
                });
            } else {
                self.resources.outcome = unavailable_descriptors();
            }
            ticks[7] = elapsed(begin);
            reads.select(TABLES[8]);
            let begin = qpc();
            if let Some(descriptors) = descriptors {
                self.exchanges.outcome = bounded(
                    cresource_exchange::walk(
                        memory,
                        base,
                        game_state,
                        descriptors,
                        &mut self.exchanges.rows,
                        &mut self.exchange_scratch,
                    ),
                    self.exchanges.rows.len(),
                );
            } else {
                self.exchanges.outcome = unavailable_descriptors();
            }
            ticks[8] = elapsed(begin);
            self.hide_off_slot_census();
        }
        if self.other_active {
            let _reads = read_accounting::table(TABLES[9]);
            let begin = qpc();
            capture_rows(&mut self.purchases, |rows| {
                cpurchase_contract::walk(memory, base, game_state, rows, &mut self.purchase_scratch)
            });
            ticks[9] = elapsed(begin);
        }
        ticks
    }

    /// One fuel walk fills CountryFuel and CountryFuelConsumers, each with its
    /// own outcome.
    fn capture_fuel(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        let outcomes = ccountry_fuel_status::walk(
            memory,
            base,
            game_state,
            &mut self.fuel.rows,
            &mut self.fuel_consumers.rows,
        );
        self.fuel.outcome = bounded(outcomes.fuel, self.fuel.rows.len());
        self.fuel_consumers.outcome = bounded(outcomes.consumers, self.fuel_consumers.rows.len());
    }

    fn set_schedule(&mut self, hour: i32, force_full: bool) {
        self.resources_active =
            replay::cadence::should_walk(TableId::CountryResources, hour, force_full);
        // The exchanges are walked with the pools; both are hourly.
        debug_assert_eq!(
            self.resources_active,
            replay::cadence::should_walk(TableId::ResourceExchanges, hour, force_full)
        );
        self.definitions_slot =
            replay::cadence::should_walk(TableId::ResourceDefinitions, hour, force_full);
        self.other_active = replay::cadence::should_walk(TableId::CountryFuel, hour, force_full);
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

    /// Off the census's slot, a census read only for its consumers is not
    /// published; a failed or truncated one stays visible so it grows and is
    /// retried.
    fn hide_off_slot_census(&mut self) {
        if !self.definitions_slot && matches!(self.definitions.outcome, Outcome::Rows { .. }) {
            self.definitions.outcome = skipped();
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
        ]
    }

    pub(crate) fn table_inputs(
        &self,
        mut projection: Option<&mut Projection<'_>>,
    ) -> [TableInput; N] {
        let outcomes = self.outcomes();
        let mut tables = [
            scalar_table::<tables::CountryFuel, _>(
                // Demand and award windows append owned values after scalar
                // conversion; compare only the final row in the encoder.
                None,
                outcomes[0],
                &self.fuel.rows,
                |r| r.country as u128,
                |r| rows::CountryFuelRow {
                    country: r.country,
                    current_raw: r.current_raw,
                    maximum_raw: r.maximum_raw,
                    gain_raw: r.gain_raw,
                    cost_raw: r.cost_raw,
                    gain_from_states_raw: r.gain_from_states_raw,
                    lend_lease_gain_raw: r.lend_lease_gain_raw,
                    lend_lease_consumption_raw: r.lend_lease_consumption_raw,
                    gain_per_oil_raw: r.gain_per_oil_raw,
                    history_index: r.history_index,
                    remaining_hours: r.remaining_hours,
                    ring_gain_raw: r.ring_gain_raw,
                    ring_consumed_raw: r.ring_consumed_raw,
                    ring_other_raw: r.ring_other_raw,
                    ring_requested_0_raw: r.ring_requested_raw[0],
                    ring_requested_1_raw: r.ring_requested_raw[1],
                    ring_requested_2_raw: r.ring_requested_raw[2],
                    ring_received_0_raw: r.ring_received_raw[0],
                    ring_received_1_raw: r.ring_received_raw[1],
                    ring_received_2_raw: r.ring_received_raw[2],
                    awards: rows::Recorded::NotRecorded,
                    demand: rows::Recorded::NotRecorded,
                    country_instance: rows::Recorded::Value(r.country_instance),
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
        ];
        if let Err(error) = super::fuel_demand::attach(&mut tables[0], &self.demand) {
            // OOM follows the existing terminal ownership policy. A named
            // demand refusal must never discard the independently known prefix.
            let _ = engine::allocation::owned_vec::<()>(Err(error));
        }
        tables
    }

    #[cfg(test)]
    pub fn tables(&self) -> [replay::frame::TableView; N] {
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

fn capture_rows<R>(captured: &mut CaptureRows<R>, mut walk: impl FnMut(&mut [R]) -> Outcome) {
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
pub(crate) use owned::{
    EconomyRestCopyContext, EconomyRestCopyReport, EconomyRestJob, copy_economy_rest_refresh,
};

#[cfg(test)]
mod tests;
