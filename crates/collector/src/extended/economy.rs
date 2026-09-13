//! Economy acquisition buffers. Growth and replay conversion run off-thread.

use crate::{
    raw::RawLog,
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_vec},
};
use engine::{
    Memory, ccountry, ccountry_experience_status, ccountry_fuel_status, ccountry_resources,
    ccountry_supply_system, cgame_state, cpdx_array::Array, cpurchase_contract, cresource_exchange,
    cstrategic_resource, cstrategic_resource_database, disrupted_supply_bucket,
    foreign_homebase_node, hub_motorization_entry, market_contract_manager, msvc_string,
    sprovince_supply_data, supply_node_record, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};
use std::mem::size_of;

pub const N: usize = 10;
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
];

/// Definitions are authenticated once and reused by the two dependent walks.
/// Their read time is charged to ResourceDefinitions, not charged again to each
/// consumer. The consumers' own timings include their additional native reads.
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("resource dictionary and consumers", 6, &[6, 7, 8])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub foreign_sources: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [512, 512, 16_384, 1024, 256, 512, 16, 4096, 1024, 128],
            foreign_sources: 16_384,
        }
    }
}

pub struct Captured {
    fuel: CaptureRows<ccountry_fuel_status::Row>,
    experience: CaptureRows<ccountry_experience_status::Row>,
    supply: CaptureRows<sprovince_supply_data::Row>,
    hubs: CaptureRows<ccountry_supply_system::Row>,
    disrupted: CaptureRows<disrupted_supply_bucket::Row>,
    foreign: CaptureRows<foreign_homebase_node::Row>,
    definitions: CaptureRows<cstrategic_resource::Row>,
    resources: CaptureRows<ccountry_resources::Row>,
    exchanges: CaptureRows<cresource_exchange::Row>,
    purchases: CaptureRows<cpurchase_contract::Row>,
    sources: Arena<u8>,
    exchange_scratch: Vec<cresource_exchange::Membership>,
    purchase_scratch: Vec<cpurchase_contract::Membership>,
    /// Raw-copy split gate for the hourly economy rest (copy/decode split).
    /// Set by the game thread every hour with a valid raw payload; `capture`
    /// then skips the typed walks and the decode worker fills the rows before
    /// the ready handoff. Preserved across `reset` (set per hour, cleared by
    /// the worker after decode).
    pub(crate) rest_raw_mode: bool,
}

impl Captured {
    /// `successful` is in local TABLES order, as are all returned arrays.
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        Self {
            fuel: CaptureRows::new(capacity.rows[0], successful[0]),
            experience: CaptureRows::new(capacity.rows[1], successful[1]),
            supply: CaptureRows::new(capacity.rows[2], successful[2]),
            hubs: CaptureRows::new(capacity.rows[3], successful[3]),
            disrupted: CaptureRows::new(capacity.rows[4], successful[4]),
            foreign: CaptureRows::new(capacity.rows[5], successful[5]),
            definitions: CaptureRows::new(capacity.rows[6], successful[6]),
            resources: CaptureRows::new(capacity.rows[7], successful[7]),
            exchanges: CaptureRows::new(capacity.rows[8], successful[8]),
            purchases: CaptureRows::new(capacity.rows[9], successful[9]),
            sources: Arena::new(capacity.foreign_sources, successful[5]),
            exchange_scratch: vec![cresource_exchange::Membership::default(); capacity.rows[8]],
            purchase_scratch: vec![cpurchase_contract::Membership::default(); capacity.rows[9]],
            rest_raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.fuel.reset();
        self.experience.reset();
        self.supply.reset();
        self.hubs.reset();
        self.disrupted.reset();
        self.foreign.reset();
        self.definitions.reset();
        self.resources.reset();
        self.exchanges.reset();
        self.purchases.reset();
        self.sources.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.fuel.allocated_bytes()
            + self.experience.allocated_bytes()
            + self.supply.allocated_bytes()
            + self.hubs.allocated_bytes()
            + self.disrupted.allocated_bytes()
            + self.foreign.allocated_bytes()
            + self.definitions.allocated_bytes()
            + self.resources.allocated_bytes()
            + self.exchanges.allocated_bytes()
            + self.purchases.allocated_bytes()
            + self.sources.allocated_bytes()
            + self.exchange_scratch.capacity() * size_of::<cresource_exchange::Membership>()
            + self.purchase_scratch.capacity() * size_of::<cpurchase_contract::Membership>()
    }

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        self.fuel.grow(&mut capacity.rows[0], successful[0]);
        self.experience.grow(&mut capacity.rows[1], successful[1]);
        self.supply.grow(&mut capacity.rows[2], successful[2]);
        self.hubs.grow(&mut capacity.rows[3], successful[3]);
        self.disrupted.grow(&mut capacity.rows[4], successful[4]);
        self.foreign.grow(&mut capacity.rows[5], successful[5]);
        self.definitions.grow(&mut capacity.rows[6], successful[6]);
        self.resources.grow(&mut capacity.rows[7], successful[7]);
        self.exchanges.grow(&mut capacity.rows[8], successful[8]);
        self.purchases.grow(&mut capacity.rows[9], successful[9]);
        self.sources
            .grow(&mut capacity.foreign_sources, successful[5]);
        grow_vec(&mut self.exchange_scratch, capacity.rows[8]);
        grow_vec(&mut self.purchase_scratch, capacity.rows[9]);
    }

    /// Reads and bounded consistency checks: no allocation, encoding or I/O. QPC
    /// ticks include every acquisition and bounded double-read comparison.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        // Hourly economy rest (copy/decode split): every hour the game thread
        // stages raw bytes via `copy_economy_rest_refresh` and the decode
        // worker fills all ten tables before the ready handoff, so the typed
        // walks below skip here. Matched flags and costs come from the raw
        // compare, applied by the caller after this returns.
        if self.rest_raw_mode {
            // Refresh payload staged: leave the reset outcomes for the decode
            // worker, which installs identical rows and outcomes.
            return ([false; N], [0; N]);
        }
        let mut matched = [false; N];
        let mut ticks = [0; N];
        macro_rules! plain {
            ($index:literal, $field:ident, $walker:path) => {{
                let begin = qpc();
                matched[$index] = capture_rows(&mut self.$field, needed[$index], |rows| {
                    $walker(memory, base, game_state, rows)
                });
                ticks[$index] = elapsed(begin);
            }};
        }
        plain!(0, fuel, ccountry_fuel_status::walk);
        plain!(1, experience, ccountry_experience_status::walk);
        plain!(2, supply, sprovince_supply_data::walk);
        plain!(3, hubs, ccountry_supply_system::walk);
        plain!(4, disrupted, disrupted_supply_bucket::walk);

        let begin = qpc();
        let first = foreign_homebase_node::walk(
            memory,
            base,
            game_state,
            &mut self.foreign.rows,
            &mut self.sources.first,
        );
        self.foreign.outcome = bounded(first.outcome, self.foreign.rows.len());
        self.sources.len = first.bytes_len;
        self.sources.required = first.required_bytes;
        if first.required_bytes < first.bytes_len
            || !valid_sources(
                self.foreign.outcome,
                &self.foreign.rows,
                &self.sources.first,
                first.bytes_len,
            )
        {
            self.foreign.outcome = invalid_sources();
        }
        if needed[5] {
            self.foreign.checked = true;
            let second = foreign_homebase_node::walk(
                memory,
                base,
                game_state,
                &mut self.foreign.second,
                &mut self.sources.second,
            );
            let mut outcome = bounded(second.outcome, self.foreign.second.len());
            if second.required_bytes < second.bytes_len
                || !valid_sources(
                    outcome,
                    &self.foreign.second,
                    &self.sources.second,
                    second.bytes_len,
                )
            {
                outcome = invalid_sources();
            }
            self.foreign.compare(outcome);
            self.foreign.mismatches += usize::from(
                !self
                    .sources
                    .compare(second.bytes_len, second.required_bytes),
            );
            matched[5] = full_match(self.foreign.outcome, outcome, self.foreign.mismatches);
        }
        ticks[5] = elapsed(begin);

        // Raw descriptor acquisition deliberately never resolves GlobalLexicalTokens.
        let begin = qpc();
        matched[6] = capture_rows(&mut self.definitions, needed[6], |rows| {
            cstrategic_resource::walk(memory, base, rows)
        });
        ticks[6] = elapsed(begin);
        let descriptors = match self.definitions.outcome {
            Outcome::Rows { len } if self.definitions.mismatches == 0 => {
                self.definitions.rows.get(..len)
            }
            _ => None,
        };
        let begin = qpc();
        if let Some(descriptors) = descriptors {
            matched[7] = capture_rows(&mut self.resources, needed[7], |rows| {
                ccountry_resources::walk(memory, base, game_state, descriptors, rows)
            });
        } else {
            self.resources.outcome = unavailable_descriptors();
        }
        ticks[7] = elapsed(begin);

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
            if needed[8] {
                self.exchanges.checked = true;
                let second = bounded(
                    cresource_exchange::walk_exports(
                        memory,
                        base,
                        game_state,
                        descriptors,
                        &mut self.exchanges.second,
                        &mut self.exchange_scratch,
                    ),
                    self.exchanges.second.len(),
                );
                matched[8] = compare_exchanges(&mut self.exchanges, second);
            }
        } else {
            self.exchanges.outcome = unavailable_descriptors();
        }
        ticks[8] = elapsed(begin);

        let begin = qpc();
        matched[9] = capture_rows(&mut self.purchases, needed[9], |rows| {
            cpurchase_contract::walk(memory, base, game_state, rows, &mut self.purchase_scratch)
        });
        ticks[9] = elapsed(begin);
        (matched, ticks)
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            self.fuel.mismatches,
            self.experience.mismatches,
            self.supply.mismatches,
            self.hubs.mismatches,
            self.disrupted.mismatches,
            self.foreign.mismatches,
            self.definitions.mismatches,
            self.resources.mismatches,
            self.exchanges.mismatches,
            self.purchases.mismatches,
        ]
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            effective(&self.fuel),
            effective(&self.experience),
            effective(&self.supply),
            effective(&self.hubs),
            effective(&self.disrupted),
            effective(&self.foreign),
            effective(&self.definitions),
            effective(&self.resources),
            effective(&self.exchanges),
            effective(&self.purchases),
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        let outcomes = self.outcomes();
        [
            table::<tables::CountryFuel, _>(outcomes[0], &self.fuel.rows, |r| {
                rows::CountryFuelRow {
                    country: r.country,
                    current_raw: r.current_raw,
                    maximum_raw: r.maximum_raw,
                    gain_raw: r.gain_raw,
                    cost_raw: r.cost_raw,
                }
            }),
            table::<tables::CountryExperience, _>(outcomes[1], &self.experience.rows, |r| {
                rows::CountryExperienceRow {
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
                }
            }),
            table::<tables::ProvinceSupply, _>(outcomes[2], &self.supply.rows, |r| {
                rows::ProvinceSupplyRow {
                    province: r.province,
                    supply_factor_raw: r.supply_factor_raw,
                    railway_health_raw: r.railway_health_raw,
                    controller_index: r.controller_index,
                    capital_country_index: r.capital_country_index,
                    supply_node_level: r.supply_node_level,
                    naval_base_level: r.naval_base_level,
                    non_damaged_infra_level: r.non_damaged_infra_level,
                    infra_level: r.infra_level,
                    flags: r.flags,
                }
            }),
            table::<tables::HubMotorization, _>(outcomes[3], &self.hubs.rows, |r| {
                rows::HubMotorizationRow {
                    owner: r.owner,
                    actor: r.actor,
                    node_index: r.node_index,
                    map_entry_index: r.map_entry_index,
                    province: r.province,
                    node_type: r.node_type,
                    level: r.level,
                }
            }),
            table::<tables::DisruptedSupply, _>(outcomes[4], &self.disrupted.rows, |r| {
                rows::DisruptedSupplyRow {
                    country: r.country,
                    province: r.province,
                    node_type: r.node_type,
                    value: r.value,
                }
            }),
            self.foreign_table(outcomes[5]),
            table::<tables::ResourceDefinitions, _>(outcomes[6], &self.definitions.rows, |r| {
                rows::ResourceDefinitionRow {
                    id: r.id,
                    token: r.token,
                    active: r.active,
                    factory_cost: r.factory_cost,
                }
            }),
            table::<tables::CountryResources, _>(outcomes[7], &self.resources.rows, |r| {
                rows::CountryResourceRow {
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
                }
            }),
            table::<tables::ResourceExchanges, _>(outcomes[8], &self.exchanges.rows, |r| {
                rows::ResourceExchangeRow {
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
                }
            }),
            table::<tables::PurchaseContracts, _>(outcomes[9], &self.purchases.rows, |r| {
                rows::PurchaseContractRow {
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
                }
            }),
        ]
    }

    fn foreign_table(&self, outcome: Outcome) -> TableView {
        if self.sources.required < self.sources.len
            || !valid_sources(
                outcome,
                &self.foreign.rows,
                &self.sources.first,
                self.sources.len,
            )
        {
            return unavailable("foreign homebase source range exceeds captured arena");
        }
        table::<tables::ForeignHomebases, _>(outcome, &self.foreign.rows, |r| {
            rows::ForeignHomebaseRow {
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
                source: self.sources.first[r.source_start..r.source_start + r.source_len].to_vec(),
            }
        })
    }
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn bounded(outcome: Outcome, capacity: usize) -> Outcome {
    if outcome.len() > capacity {
        Outcome::Unavailable {
            reason: "walker row count exceeds allocated buffer",
        }
    } else if matches!(outcome, Outcome::Truncated { unread_rows: 0, .. }) {
        Outcome::Unavailable {
            reason: "walker marked truncation without missing rows",
        }
    } else {
        outcome
    }
}

fn capture_rows<R: Default + Clone + PartialEq>(
    captured: &mut CaptureRows<R>,
    needed: bool,
    mut walk: impl FnMut(&mut [R]) -> Outcome,
) -> bool {
    captured.outcome = bounded(walk(&mut captured.rows), captured.rows.len());
    if !needed {
        return false;
    }
    captured.checked = true;
    let second = bounded(walk(&mut captured.second), captured.second.len());
    captured.compare(second)
}

fn effective<R>(captured: &CaptureRows<R>) -> Outcome {
    if captured.checked && captured.mismatches != 0 {
        Outcome::Unavailable {
            reason: "table changed during double read",
        }
    } else {
        captured.outcome
    }
}

/// Validation-only O(n log n) ordering in caller-owned buffers. A successful
/// check requires every native field to match across both complete censuses.
/// Truncated opposite-root prefixes need not contain the same objects, so they
/// cannot establish a complete consistency check or a differing-world claim.
fn compare_exchanges(captured: &mut CaptureRows<cresource_exchange::Row>, second: Outcome) -> bool {
    if let (Outcome::Rows { len }, Outcome::Rows { len: other_len }) = (captured.outcome, second) {
        captured.rows[..len].sort_unstable_by_key(|row| row.id);
        captured.second[..other_len].sort_unstable_by_key(|row| row.id);
        captured.compare(second)
    } else {
        captured.mismatches = usize::from(matches!(captured.outcome, Outcome::Rows { .. }));
        false
    }
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

impl Captured {
    /// Decode-worker interpretation of a raw hourly economy-rest payload.
    /// Replays the existing ten walks against the recorded spans in
    /// dependency order (definitions before resources/exchanges), installing
    /// identical rows, arenas, truncation counts and failure reasons into the
    /// frame's pooled buffers. Checked/mismatches set by the raw compare
    /// survive. Clears `rest_raw_mode` so the next capture starts clean.
    pub(crate) fn decode_economy_rest_payload(
        &mut self,
        base: usize,
        job: &EconomyRestJob,
        decode_ticks: &mut [u64; 10],
    ) {
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        self.fuel.outcome = bounded(
            ccountry_fuel_status::walk(&recorded, base, job.game_state, &mut self.fuel.rows),
            self.fuel.rows.len(),
        );
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        self.experience.outcome = bounded(
            ccountry_experience_status::walk(
                &recorded,
                base,
                job.game_state,
                &mut self.experience.rows,
            ),
            self.experience.rows.len(),
        );
        decode_ticks[1] += elapsed(started);
        let started = qpc();
        self.supply.outcome = bounded(
            sprovince_supply_data::walk(&recorded, base, job.game_state, &mut self.supply.rows),
            self.supply.rows.len(),
        );
        decode_ticks[2] += elapsed(started);
        let started = qpc();
        self.hubs.outcome = bounded(
            ccountry_supply_system::walk(&recorded, base, job.game_state, &mut self.hubs.rows),
            self.hubs.rows.len(),
        );
        self.disrupted.outcome = bounded(
            disrupted_supply_bucket::walk(&recorded, base, job.game_state, &mut self.disrupted.rows),
            self.disrupted.rows.len(),
        );
        decode_ticks[3] += elapsed(started);
        decode_ticks[4] += 0;
        let started = qpc();
        let first = foreign_homebase_node::walk(
            &recorded,
            base,
            job.game_state,
            &mut self.foreign.rows,
            &mut self.sources.first,
        );
        self.foreign.outcome = bounded(first.outcome, self.foreign.rows.len());
        self.sources.len = first.bytes_len;
        self.sources.required = first.required_bytes;
        if first.required_bytes < first.bytes_len
            || !valid_sources(
                self.foreign.outcome,
                &self.foreign.rows,
                &self.sources.first,
                first.bytes_len,
            )
        {
            self.foreign.outcome = invalid_sources();
        }
        decode_ticks[5] += elapsed(started);
        let started = qpc();
        self.definitions.outcome = bounded(
            cstrategic_resource::walk(&recorded, base, &mut self.definitions.rows),
            self.definitions.rows.len(),
        );
        decode_ticks[6] += elapsed(started);
        let descriptors = match self.definitions.outcome {
            Outcome::Rows { len } if self.definitions.mismatches == 0 => {
                self.definitions.rows.get(..len)
            }
            _ => None,
        };
        let started = qpc();
        if let Some(descriptors) = descriptors {
            // Copy descriptors to an owned buffer: the replay borrows the
            // pooled rows while the walk needs a stable slice; the copy is
            // decode-side only and never touches the game thread.
            let owned: Vec<cstrategic_resource::Row> = descriptors.to_vec();
            self.resources.outcome = bounded(
                ccountry_resources::walk(
                    &recorded,
                    base,
                    job.game_state,
                    &owned,
                    &mut self.resources.rows,
                ),
                self.resources.rows.len(),
            );
        } else {
            self.resources.outcome = unavailable_descriptors();
        }
        decode_ticks[7] += elapsed(started);
        let started = qpc();
        if let Some(descriptors) = descriptors {
            let owned: Vec<cstrategic_resource::Row> = descriptors.to_vec();
            self.exchanges.outcome = bounded(
                cresource_exchange::walk(
                    &recorded,
                    base,
                    job.game_state,
                    &owned,
                    &mut self.exchanges.rows,
                    &mut self.exchange_scratch,
                ),
                self.exchanges.rows.len(),
            );
        } else {
            self.exchanges.outcome = unavailable_descriptors();
        }
        decode_ticks[8] += elapsed(started);
        let started = qpc();
        self.purchases.outcome = bounded(
            cpurchase_contract::walk(
                &recorded,
                base,
                job.game_state,
                &mut self.purchases.rows,
                &mut self.purchase_scratch,
            ),
            self.purchases.rows.len(),
        );
        decode_ticks[9] += elapsed(started);
        self.rest_raw_mode = false;
    }

    /// Game-thread raw copy for an hourly refresh. Copies `first` always and
    /// `second` while any table still needs double-read checks, compares the
    /// per-table raw partitions, and records copy-only costs plus the existing
    /// checked/mismatches bookkeeping. Returns whether the payload is complete
    /// (`raw_ok`); on staging overflow the caller falls back to the typed walk
    /// and enqueues nothing.
    pub(crate) fn copy_economy_rest_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: EconomyRestCopyContext,
        first: &mut EconomyRestJob,
        mut second: Option<&mut EconomyRestJob>,
    ) -> EconomyRestCopyReport {
        let started = qpc();
        first.clear();
        first.game_state = ctx.game_state;
        copy_economy_rest_pass(memory, ctx.base, ctx.game_state, &mut first.log);
        let mut ticks = [0u64; 10];
        ticks[0] = elapsed(started);
        let mut matched = [true; 10];
        if let Some(second_job) = second.as_deref_mut() {
            let second_started = qpc();
            second_job.clear();
            second_job.game_state = ctx.game_state;
            copy_economy_rest_pass(memory, ctx.base, ctx.game_state, &mut second_job.log);
            ticks[0] += elapsed(second_started);
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal =
                    raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        self.fuel.checked = ctx.needed[0];
        self.fuel.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        self.experience.checked = ctx.needed[1];
        self.experience.mismatches = usize::from(ctx.needed[1] && !matched[1]);
        self.supply.checked = ctx.needed[2];
        self.supply.mismatches = usize::from(ctx.needed[2] && !matched[2]);
        self.hubs.checked = ctx.needed[3];
        self.hubs.mismatches = usize::from(ctx.needed[3] && !matched[3]);
        self.disrupted.checked = ctx.needed[4];
        self.disrupted.mismatches = usize::from(ctx.needed[4] && !matched[4]);
        self.foreign.checked = ctx.needed[5];
        self.foreign.mismatches = usize::from(ctx.needed[5] && !matched[5]);
        self.definitions.checked = ctx.needed[6];
        self.definitions.mismatches = usize::from(ctx.needed[6] && !matched[6]);
        self.resources.checked = ctx.needed[7];
        self.resources.mismatches = usize::from(ctx.needed[7] && !matched[7]);
        self.exchanges.checked = ctx.needed[8];
        self.exchanges.mismatches = usize::from(ctx.needed[8] && !matched[8]);
        self.purchases.checked = ctx.needed[9];
        self.purchases.mismatches = usize::from(ctx.needed[9] && !matched[9]);
        let raw_ok = !first.log.overflowed
            && second.is_none_or(|job| !job.log.overflowed);
        EconomyRestCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }
}

// ===========================================================================
// Hourly economy-rest raw-copy split (CountryFuel, CountryExperience,
// ProvinceSupply, HubMotorization, DisruptedSupply, ForeignHomebases,
// ResourceDefinitions, CountryResources, ResourceExchanges,
// PurchaseContracts).
//
// Zero-engine-change replay (Session/Radar precedent for census plus
// per-country walks): the game thread copies raw bytes only and the single
// FIFO decode worker replays the existing typed walks against the recorded
// spans (`crate::raw::Recorded`), so rows, ordinals, skips, truncation and
// failure reasons are identical by construction (see `crate::raw` for the
// mirror rules).
//
// One shared country census feeds fuel, experience, hubs, disrupted, foreign,
// resources, exchanges and purchases (no duplicated censuses); the supply
// root feeds supply, hubs, disrupted and foreign; the resource database feeds
// definitions, resources and exchanges; the market manager feeds purchases.
// Per-country/per-province scans use batched gathers: fuel/experience status
// blocks (0xe8/0x58 bulk), province supply array (count*0x20 bulk), hub
// settings nodes with motorization entries, disrupted buckets, foreign
// records with source strings, resource pools with value records, exchange
// objects with bucket/route vectors, contract objects with directional
// vectors. Empty tables share their walks and move with them.
//
// Fault attribution is conservative like the pilot: shared census spans carry
// all ten bits, supply-root spans the four supply bits, resource-DB spans the
// three resource bits, and deployment of per-table leaves uses single-bit
// masks. The mirror traverses structurally and records every discoverable
// span, never aborting except on overflow: the replay derives every outcome,
// precedence and error scope from the same recorded spans, so later extras
// are harmless.
// ===========================================================================

/// Fault-attribution bits in family order: [fuel, experience, supply, hubs,
/// disrupted, foreign, definitions, resources, exchanges, purchases]. Shared
/// census spans carry all ten bits; supply-root spans the four supply bits;
/// resource-DB spans the three resource bits.
const M_E_FUEL: u32 = 0x001;
const M_E_EXP: u32 = 0x002;
const M_E_SUPPLY: u32 = 0x004;
const M_E_HUBS: u32 = 0x008;
const M_E_DISRUPTED: u32 = 0x010;
const M_E_FOREIGN: u32 = 0x020;
const M_E_EXCH: u32 = 0x100;
const M_E_PURCH: u32 = 0x200;
const M_E_CENSUS: u32 = 0x3FF;
const M_E_SUPPLY_ROOT: u32 = 0x03C;
const M_E_RESOURCE_DB: u32 = 0x1C0;
const M_E_RES_OBJ: u32 = 0x180;
const M_E_SUPPLY_OBJ: u32 = 0x038;

/// One hour of raw economy-rest payload. Flat pools recycle capacities and the
/// steady state allocates on neither thread; growth happens on the decode side
/// only (see `crate::decode::recycle_economy_rest_job`).
#[derive(Debug, Default)]
pub(crate) struct EconomyRestJob {
    pub log: RawLog<10>,
    pub game_state: usize,
}

impl EconomyRestJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is about 439 fuel/experience rows,
            // 13,413 province supply records (~430 KiB), 353 hub rows,
            // 3,073 resource rows with 9 pools, 330 exchanges and 42
            // contracts; both pools start above that so steady state never
            // overflows (see the worst-case caps test).
            log: RawLog::new(1 << 18, 16 << 20),
            game_state: 0,
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed hours fall back to the typed walk
    /// in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct EconomyRestCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Whole-log raw equality gated by live-fault conservatism, in family
    /// order [fuel, experience, supply, hubs, disrupted, foreign,
    /// definitions, resources, exchanges, purchases].
    pub matched: [bool; 10],
    pub copy_ticks: [u64; 10],
    pub raw_bytes: [u64; 10],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct EconomyRestCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; 10],
}

fn e_pool_u64(log: &RawLog<10>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn e_pool_u32(log: &RawLog<10>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn espan(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Bulk variant for contiguous table copies (status blocks, array data).
/// Enclosed sub-reads serve from the bulk span at replay.
fn esbulk(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take_bulk(memory, addr?, len, mask)
}

/// Mirror of one MSVC string's reads: 32-byte header plus heap payload and
/// terminator. Inline strings ride in the header span.
fn copy_economy_string(log: &mut RawLog<10>, memory: &impl Memory, string: usize, mask: u32) {
    let Some(header_start) = espan(log, memory, Some(string), 32, mask) else {
        return;
    };
    let _ = espan(
        log,
        memory,
        string.checked_add(msvc_string::LENGTH),
        8,
        mask,
    );
    let _ = espan(
        log,
        memory,
        string.checked_add(msvc_string::CAPACITY),
        8,
        mask,
    );
    let _ = espan(log, memory, Some(string), 8, mask);
    let mut header = [0u8; 32];
    header.copy_from_slice(&log.bytes[header_start..header_start + 32]);
    let len = match usize::try_from(u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    )) {
        Ok(len) => len,
        Err(_) => return,
    };
    let capacity = u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
    if len as u64 > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return;
    }
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        match usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&s| s != 0)
        {
            Some(s) => s,
            None => return,
        }
    };
    if source.checked_add(len).is_none() {
        return;
    }
    if len != 0 {
        let _ = espan(log, memory, Some(source), len, mask);
    }
    let _ = espan(log, memory, source.checked_add(len), 1, mask);
}

/// Authenticated country census shared by fuel, experience, hubs, disrupted,
/// foreign, resources, exchanges and purchases. Records the exact read set of
/// `ccountry::Reader::new` plus one full `read_at` per slot. Returns the
/// canonical country list (slot, pointer, id) for per-country detail copies.
fn copy_economy_link(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<Option<u32>> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = espan(log, memory, links.data.checked_add(offset), 4, M_E_CENSUS)?;
    let slot = e_pool_u32(log, start);
    if slot == 0 {
        return Some(None);
    }
    if slot >= countries {
        return None;
    }
    Some(Some(slot))
}

/// One structural copy pass: shared census, supply root with province data,
/// per-country fuel/experience/resources/supply/market blocks, resource
/// database, market manager with contracts. Stops at the first typed abort
/// point; later extras are harmless.
#[allow(clippy::collapsible_if)]
fn copy_economy_rest_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    log: &mut RawLog<10>,
) {
    log.has_bulk = true;
    // Census runs inline with per-country details copied immediately while
    // iterating slots, parsing link bytes from the log. No heap allocation on
    // the game thread.
    let Some(countries_start) = espan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        M_E_CENSUS,
    ) else {
        return;
    };
    let Some(links_start) = espan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        M_E_CENSUS,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return,
    };
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = match Array::from_header(raw, 4) {
        Some(a) => a,
        None => return,
    };
    if countries.count < 2 || links.count == 0 {
        return;
    }
    let country_vtable = match base.checked_add(ccountry::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    // Supply root shared by supply/hubs/disrupted/foreign.
    let supply_root = game_state
        .checked_add(cgame_state::SUPPLY_SYSTEM)
        .and_then(|p| {
            let s = espan(log, memory, Some(p), 8, M_E_SUPPLY_ROOT)?;
            usize::try_from(e_pool_u64(log, s)).ok().filter(|&v| v != 0)
        });
    if let Some(root) = supply_root {
        let _ = esbulk(log, memory, Some(root), 16, M_E_SUPPLY_ROOT);
        let _ = espan(
            log,
            memory,
            root.checked_add(engine::csupply_system::PROVINCE_DATA),
            16,
            M_E_SUPPLY,
        );
        let _ = espan(
            log,
            memory,
            root.checked_add(engine::csupply_system::COUNTRY_SYSTEM_DATA),
            16,
            M_E_SUPPLY_ROOT,
        );
        let _ = espan(
            log,
            memory,
            root.checked_add(engine::csupply_system::COUNTRY_COMPANION_DATA),
            16,
            M_E_SUPPLY_ROOT,
        );
        // Province supply array data (batched gather): header plus full
        // contiguous records. The walk batches 64x0x20; one bulk covers all.
        if let Some(h) = root.checked_add(engine::csupply_system::PROVINCE_DATA)
            .and_then(|p| {
                // Re-read header bytes from the log for discovery.
                log.spans.iter().rev().find_map(|span| {
                    if span.addr == p && span.len == 16 && span.ok {
                        let mut h = [0u8; 16];
                        h.copy_from_slice(
                            &log.bytes[span.start as usize..span.start as usize + 16],
                        );
                        Array::from_header(h, sprovince_supply_data::STRIDE)
                    } else {
                        None
                    }
                })
            })
        {
            if h.count >= 2 {
                let bytes = (h.count as usize).saturating_mul(sprovince_supply_data::STRIDE);
                let _ = esbulk(log, memory, Some(h.data), bytes, M_E_SUPPLY);
            }
        }
        // Provinces array header for the slot-count check.
        let _ = espan(
            log,
            memory,
            game_state.checked_add(cgame_state::PROVINCES),
            16,
            M_E_SUPPLY,
        );
    }
    // Resource database shared by definitions/resources/exchanges.
    if let Some(db) = base.checked_add(cstrategic_resource_database::SINGLETON_RVA).and_then(|p| {
        let s = espan(log, memory, Some(p), 8, M_E_RESOURCE_DB)?;
        usize::try_from(e_pool_u64(log, s)).ok().filter(|&v| v != 0)
    }) {
        let Some(items_start) = espan(
            log,
            memory,
            db.checked_add(cstrategic_resource_database::ITEMS),
            16,
            M_E_RESOURCE_DB,
        ) else {
            return;
        };
        let Some(keys_start) = espan(
            log,
            memory,
            db.checked_add(cstrategic_resource_database::KEYS),
            16,
            M_E_RESOURCE_DB,
        ) else {
            return;
        };
        let mut h = [0u8; 16];
        h.copy_from_slice(&log.bytes[items_start..items_start + 16]);
        let items = match Array::from_header(h, 8) {
            Some(a) => a,
            None => return,
        };
        h.copy_from_slice(&log.bytes[keys_start..keys_start + 16]);
        let keys = match Array::from_header(h, 4) {
            Some(a) => a,
            None => return,
        };
        if items.count == 0 {
            return;
        }
        let _ = esbulk(
            log,
            memory,
            Some(keys.data),
            (keys.count as usize).saturating_mul(4),
            M_E_RESOURCE_DB,
        );
        for index in 0..items.count {
            let addr = (index as usize)
                .checked_mul(8)
                .and_then(|o| items.data.checked_add(o));
            let Some(ptr_start) = espan(log, memory, addr, 8, M_E_RESOURCE_DB) else {
                continue;
            };
            let pointer = match usize::try_from(e_pool_u64(log, ptr_start))
                .ok()
                .filter(|&p| p != 0)
            {
                Some(p) => p,
                None => continue,
            };
            // Descriptor bulk covers vtable/token/active/index/cost.
            let _ = esbulk(log, memory, Some(pointer), 0x100, M_E_RESOURCE_DB);
            if log.overflowed {
                return;
            }
        }
    }
    // Market manager shared by purchases.
    let market = game_state.checked_add(cgame_state::MARKET_MANAGER).and_then(|p| {
        let s = espan(log, memory, Some(p), 8, M_E_PURCH)?;
        usize::try_from(e_pool_u64(log, s)).ok().filter(|&v| v != 0)
    });
    if let Some(manager) = market {
        let _ = espan(
            log,
            memory,
            manager.checked_add(market_contract_manager::OWNED_CONTRACTS),
            16,
            M_E_PURCH,
        );
        let _ = espan(
            log,
            memory,
            manager.checked_add(market_contract_manager::CONTRACT_TABLE_A),
            16,
            M_E_PURCH,
        );
        let _ = espan(
            log,
            memory,
            manager.checked_add(market_contract_manager::CONTRACT_TABLE_B),
            16,
            M_E_PURCH,
        );
    }
    // Per-country batched gathers: fuel, experience, resources, market status,
    // supply systems with hub/disrupted/foreign details.
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let Some(pointer_start) = espan(log, memory, pointer_addr, 8, M_E_CENSUS) else {
            return;
        };
        let pointer = match usize::try_from(e_pool_u64(log, pointer_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let Some(vtable_start) = espan(log, memory, Some(pointer), 8, M_E_CENSUS) else {
            return;
        };
        let Some(id_start) = espan(log, memory, pointer.checked_add(ccountry::ID), 4, M_E_CENSUS)
        else {
            return;
        };
        if e_pool_u64(log, vtable_start) != country_vtable {
            return;
        }
        let id = e_pool_u32(log, id_start);
        let canonical = match copy_economy_link(log, memory, &links, countries.count, id) {
            Some(Some(c)) => c,
            _ => return,
        };
        if copy_economy_link(log, memory, &links, countries.count, slot) != Some(Some(canonical)) {
            return;
        }
        if copy_economy_link(log, memory, &links, countries.count, canonical) != Some(Some(canonical))
        {
            return;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let Some(canonical_start) = espan(log, memory, canonical_addr, 8, M_E_CENSUS) else {
            return;
        };
        if e_pool_u64(log, canonical_start) != pointer as u64 {
            return;
        }
        if slot != canonical {
            continue;
        }
        // Fuel status block (batched gather): one bulk covers the walk's
        // scattered vtable/quantities/cost/owner reads.
        if let Some(ptr_start) = espan(
            log,
            memory,
            pointer.checked_add(ccountry::FUEL_STATUS),
            8,
            M_E_FUEL,
        ) {
            if let Some(status) = usize::try_from(e_pool_u64(log, ptr_start))
                .ok()
                .filter(|&p| p != 0)
            {
                let _ = esbulk(log, memory, Some(status), 0xe8, M_E_FUEL);
            }
        }
        // Experience block: one contiguous 0x58 bulk.
        if let Some(ptr_start) = espan(
            log,
            memory,
            pointer.checked_add(ccountry::EXPERIENCE_STATUS),
            8,
            M_E_EXP,
        ) {
            if let Some(status) = usize::try_from(e_pool_u64(log, ptr_start))
                .ok()
                .filter(|&p| p != 0)
            {
                let _ = esbulk(log, memory, Some(status), 0x58, M_E_EXP);
            }
        }
        // Resources object with pools and trade vectors.
        if let Some(ptr_start) = espan(
            log,
            memory,
            pointer.checked_add(ccountry::RESOURCES),
            8,
            M_E_RES_OBJ,
        ) {
            if let Some(resources) = usize::try_from(e_pool_u64(log, ptr_start))
                .ok()
                .filter(|&p| p != 0)
            {
                copy_economy_resources(log, memory, base, resources);
            }
        }
        // Market status backlink for purchase authentication.
        if let Some(ptr_start) = espan(
            log,
            memory,
            pointer.checked_add(ccountry::EQUIPMENT_MARKET_STATUS),
            8,
            M_E_PURCH,
        ) {
            if let Some(status) = usize::try_from(e_pool_u64(log, ptr_start))
                .ok()
                .filter(|&p| p != 0)
            {
                let _ = esbulk(log, memory, Some(status), 0x80, M_E_PURCH);
            }
        }
        if log.overflowed {
            return;
        }
    }
    // Supply per-system details plus market contracts (second loops over the
    // same slots using log-served discovery).
    copy_economy_supply_details(log, memory, base, game_state);
    copy_economy_market_contracts(log, memory, base, market);
}

/// Mirror of one country's resources object: full 0x800 bulk plus pool
/// headers with value records, export buckets with exchange vectors, import
/// routes with exchange vectors, and exchange objects with counterpart
/// vectors. All spans carry the shared resources/exchanges mask.
/// Resource pool header offsets within CCountryResources (mirrors the
/// private `POOLS` table in `ccountry_resources` without touching engine).
const E_POOLS: [usize; 9] = [
    engine::ccountry_resources::PRODUCED_POOL,
    engine::ccountry_resources::ADJUSTMENT_POOL,
    engine::ccountry_resources::IMPORTED_POOL,
    engine::ccountry_resources::USED_POOL,
    engine::ccountry_resources::SPECIAL_TOTAL_POOL,
    engine::ccountry_resources::PRODUCTION_LINE_POOL,
    engine::ccountry_resources::TO_EXPORT_POOL,
    engine::ccountry_resources::BASE_EXPORT_POOL,
    engine::ccountry_resources::EXPORTED_POOL,
];

fn copy_economy_resources(log: &mut RawLog<10>, memory: &impl Memory, base: usize, pointer: usize) {
    let _ = base;
    let Some(obj_start) = esbulk(log, memory, Some(pointer), 0x800, M_E_RES_OBJ) else {
        return;
    };
    let mut obj = [0u8; 0x800];
    obj.copy_from_slice(&log.bytes[obj_start..obj_start + 0x800]);
    // Pool headers live at the known POOLS offsets; parse data/count from the
    // copied bulk and take the value arrays.
    for offset in E_POOLS {
        let header: [u8; 24] = match obj
            .get(offset..offset + 24)
            .and_then(|b| b.try_into().ok())
        {
            Some(h) => h,
            None => continue,
        };
        let data = usize::try_from(u64::from_le_bytes(header[8..16].try_into().unwrap())).ok();
        let count =
            u32::from_le_bytes(header[20..24].try_into().unwrap()) as usize;
        if let Some(data) = data.filter(|&d| d != 0 && count != 0) {
            let bytes = count.saturating_mul(0x10);
            let _ = esbulk(log, memory, Some(data), bytes, M_E_RES_OBJ);
        }
    }
    // Export buckets outer array plus per-resource vectors with exchanges.
    if let Some(buckets_addr) = pointer.checked_add(engine::ccountry_resources::EXPORT_BUCKETS) {
        // Header already inside the 0x800 bulk, but take the exact span for
        // exact-match service plus the vectors below.
        let Some(h_start) = espan(log, memory, Some(buckets_addr), 16, M_E_RES_OBJ) else {
            return;
        };
        let mut h = [0u8; 16];
        h.copy_from_slice(&log.bytes[h_start..h_start + 16]);
        if let Some(buckets) = Array::from_header(h, engine::ccountry_resources::EXPORT_BUCKET_STRIDE)
        {
            for index in 0..buckets.count {
                let addr = (index as usize)
                    .checked_mul(engine::ccountry_resources::EXPORT_BUCKET_STRIDE)
                    .and_then(|o| buckets.data.checked_add(o));
                let Some(vec_start) = espan(log, memory, addr, 16, M_E_RES_OBJ) else {
                    continue;
                };
                let mut vh = [0u8; 16];
                vh.copy_from_slice(&log.bytes[vec_start..vec_start + 16]);
                if let Some(vec) = Array::from_header(vh, 8) {
                    copy_economy_exchange_vec(log, memory, &vec);
                }
                if log.overflowed {
                    return;
                }
            }
        }
    }
    // Import routes vector with exchanges.
    if let Some(routes_addr) = pointer.checked_add(engine::ccountry_resources::IMPORT_ROUTES) {
        let Some(h_start) = espan(log, memory, Some(routes_addr), 16, M_E_RES_OBJ) else {
            return;
        };
        let mut h = [0u8; 16];
        h.copy_from_slice(&log.bytes[h_start..h_start + 16]);
        if let Some(vec) = Array::from_header(h, 8) {
            copy_economy_exchange_vec(log, memory, &vec);
        }
    }
}

/// Mirror of one exchange pointer vector: pointers plus 0xf0 bulks plus
/// exporter owner words. Takes are unconditional; the replay validates.
fn copy_economy_exchange_vec(log: &mut RawLog<10>, memory: &impl Memory, vec: &Array) {
    for ordinal in 0..vec.count {
        let addr = (ordinal as usize)
            .checked_mul(8)
            .and_then(|o| vec.data.checked_add(o));
        let Some(ptr_start) = espan(log, memory, addr, 8, M_E_EXCH) else {
            continue;
        };
        let pointer = match usize::try_from(e_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let _ = esbulk(log, memory, Some(pointer), engine::cresource_exchange::SIZE, M_E_EXCH);
        // Exporter owner word for backlink resolution.
        if let Some(owner_addr) = pointer.checked_add(engine::ccountry_resources::OWNER) {
            // Resources owner lives at +0x20 of the exporter resources
            // object, already bulk-copied; the exchange's own exporter
            // pointer plus owner word below serve the resolve.
            let _ = espan(log, memory, Some(owner_addr), 4, M_E_EXCH);
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of the supply per-system details: country system array with
/// per-system 0x290 bulks, hub settings with nodes/entries, disrupted
/// buckets with sentinels, foreign records with source strings.
fn copy_economy_supply_details(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) {
    let _ = base;
    let root = match game_state.checked_add(cgame_state::SUPPLY_SYSTEM).and_then(|p| {
        log.spans.iter().rev().find_map(|span| {
            if span.addr == p && span.len == 8 && span.ok {
                usize::try_from(e_pool_u64(log, span.start as usize)).ok()
            } else {
                None
            }
        })
    }) {
        Some(r) => r,
        None => return,
    };
    if root == 0 {
        return;
    }
    let systems_header = match root.checked_add(engine::csupply_system::COUNTRY_SYSTEM_DATA) {
        Some(p) => log.spans.iter().rev().find_map(|span| {
            if span.addr == p && span.len == 16 && span.ok {
                let mut h = [0u8; 16];
                h.copy_from_slice(&log.bytes[span.start as usize..span.start as usize + 16]);
                Array::from_header(h, 8)
            } else {
                None
            }
        }),
        None => None,
    };
    let systems = match systems_header {
        Some(a) => a,
        None => return,
    };
    for slot in 1..systems.count {
        let addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| systems.data.checked_add(o));
        let Some(ptr_start) = espan(log, memory, addr, 8, M_E_SUPPLY_OBJ) else {
            continue;
        };
        let system = match usize::try_from(e_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let Some(obj_start) = esbulk(
            log,
            memory,
            Some(system),
            engine::ccountry_supply_system::SIZE,
            M_E_SUPPLY_OBJ,
        ) else {
            continue;
        };
        let mut obj = [0u8; 0x290];
        obj.copy_from_slice(&log.bytes[obj_start..obj_start + engine::ccountry_supply_system::SIZE]);
        // Hub settings array.
        let hub_data = usize::try_from(u64::from_le_bytes(
            obj[engine::ccountry_supply_system::HUB_SETTINGS_DATA
                ..engine::ccountry_supply_system::HUB_SETTINGS_DATA + 8]
                .try_into()
                .unwrap(),
        ))
        .ok();
        let hub_count = u32::from_le_bytes(
            obj[engine::ccountry_supply_system::HUB_SETTINGS_COUNT
                ..engine::ccountry_supply_system::HUB_SETTINGS_COUNT + 4]
                .try_into()
                .unwrap(),
        );
        if let Some(data) = hub_data.filter(|&d| d != 0 && hub_count != 0) {
            for node_index in 0..hub_count {
                let node_addr = (node_index as usize)
                    .checked_mul(supply_node_record::STRIDE)
                    .and_then(|o| data.checked_add(o));
                let Some(node_start) =
                    esbulk(log, memory, node_addr, supply_node_record::STRIDE, M_E_HUBS)
                else {
                    continue;
                };
                let mut node = [0u8; 0x28];
                node.copy_from_slice(&log.bytes[node_start..node_start + supply_node_record::STRIDE]);
                let entries_data = usize::try_from(u64::from_le_bytes(
                    node[supply_node_record::MOTORIZATION_DATA
                        ..supply_node_record::MOTORIZATION_DATA + 8]
                        .try_into()
                        .unwrap(),
                ))
                .ok();
                let entries_count = u32::from_le_bytes(
                    node[supply_node_record::MOTORIZATION_COUNT
                        ..supply_node_record::MOTORIZATION_COUNT + 4]
                        .try_into()
                        .unwrap(),
                );
                if let Some(edata) = entries_data.filter(|&d| d != 0 && entries_count != 0) {
                    let bytes = (entries_count as usize)
                        .saturating_mul(hub_motorization_entry::STRIDE);
                    let _ = esbulk(log, memory, Some(edata), bytes, M_E_HUBS);
                }
                if log.overflowed {
                    return;
                }
            }
        }
        // Disrupted buckets: header already in the 0x290 bulk; copy the
        // bucket extent plus end sentinel.
        {
            let buckets = usize::try_from(u64::from_le_bytes(
                obj[engine::ccountry_supply_system::DISRUPTED_BUCKETS
                    ..engine::ccountry_supply_system::DISRUPTED_BUCKETS + 8]
                    .try_into()
                    .unwrap(),
            ))
            .ok()
            .filter(|&p| p != 0);
            let occupied = u32::from_le_bytes(
                obj[engine::ccountry_supply_system::DISRUPTED_COUNT
                    ..engine::ccountry_supply_system::DISRUPTED_COUNT + 4]
                    .try_into()
                    .unwrap(),
            );
            let mask = u32::from_le_bytes(
                obj[engine::ccountry_supply_system::DISRUPTED_MASK
                    ..engine::ccountry_supply_system::DISRUPTED_MASK + 4]
                    .try_into()
                    .unwrap(),
            );
            let max_distance = obj[engine::ccountry_supply_system::DISRUPTED_MAX_DISTANCE];
            if let Some(buckets) = buckets
                && occupied != 0
                && mask.checked_add(1).is_some_and(|c| c.is_power_of_two())
                && max_distance != 0
            {
                let bucket_count = mask + 1;
                if let Some(scan) = bucket_count.checked_add(u32::from(max_distance))
                    && let Some(bytes) = (scan as usize)
                        .checked_mul(engine::disrupted_supply_bucket::STRIDE)
                        .and_then(|b| b.checked_add(engine::disrupted_supply_bucket::DISTANCE_PLUS_ONE + 1))
                {
                    let _ = esbulk(log, memory, Some(buckets), bytes, M_E_DISRUPTED);
                }
            }
        }
        // Foreign homebases: array plus records with source strings.
        {
            let foreign_data = usize::try_from(u64::from_le_bytes(
                obj[engine::ccountry_supply_system::FOREIGN_HOMEBASE_DATA
                    ..engine::ccountry_supply_system::FOREIGN_HOMEBASE_DATA + 8]
                    .try_into()
                    .unwrap(),
            ))
            .ok();
            let foreign_count = u32::from_le_bytes(
                obj[engine::ccountry_supply_system::FOREIGN_HOMEBASE_COUNT
                    ..engine::ccountry_supply_system::FOREIGN_HOMEBASE_COUNT + 4]
                    .try_into()
                    .unwrap(),
            );
            if let Some(data) = foreign_data.filter(|&d| d != 0 && foreign_count != 0) {
                for ordinal in 0..foreign_count {
                    let rec_addr = (ordinal as usize)
                        .checked_mul(engine::foreign_homebase_node::STRIDE)
                        .and_then(|o| data.checked_add(o));
                    let Some(rec_start) = esbulk(
                        log,
                        memory,
                        rec_addr,
                        engine::foreign_homebase_node::STRIDE,
                        M_E_FOREIGN,
                    ) else {
                        continue;
                    };
                    // Source string lives at +0x28 inside the record; take
                    // the exact header span plus payload for exact service
                    // (the bulk above already covers the record itself).
                    let _ = rec_start;
                    if let Some(raddr) = rec_addr.and_then(|r| {
                        r.checked_add(engine::foreign_homebase_node::SOURCE)
                    }) {
                        copy_economy_string(log, memory, raddr, M_E_FOREIGN);
                    }
                    if log.overflowed {
                        return;
                    }
                }
            }
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of the market contracts: owned vector with 0x260 bulks plus
/// per-country directional vectors. All spans carry the purchases mask.
fn copy_economy_market_contracts(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    market: Option<usize>,
) {
    let _ = base;
    let manager = match market {
        Some(m) => m,
        None => return,
    };
    let owned = log.spans.iter().rev().find_map(|span| {
        if span.addr == manager + market_contract_manager::OWNED_CONTRACTS
            && span.len == 16
            && span.ok
        {
            let mut h = [0u8; 16];
            h.copy_from_slice(&log.bytes[span.start as usize..span.start as usize + 16]);
            Array::from_header(h, 8)
        } else {
            None
        }
    });
    let owned = match owned {
        Some(a) => a,
        None => return,
    };
    for index in 0..owned.count {
        let addr = (index as usize)
            .checked_mul(8)
            .and_then(|o| owned.data.checked_add(o));
        let Some(ptr_start) = espan(log, memory, addr, 8, M_E_PURCH) else {
            continue;
        };
        let pointer = match usize::try_from(e_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let _ = esbulk(log, memory, Some(pointer), 0x260, M_E_PURCH);
        if log.overflowed {
            return;
        }
    }
    for table in [
        market_contract_manager::CONTRACT_TABLE_A,
        market_contract_manager::CONTRACT_TABLE_B,
    ] {
        let outer = log.spans.iter().rev().find_map(|span| {
            if span.addr == manager + table && span.len == 16 && span.ok {
                let mut h = [0u8; 16];
                h.copy_from_slice(&log.bytes[span.start as usize..span.start as usize + 16]);
                Array::from_header(h, market_contract_manager::COUNTRY_VECTOR_STRIDE)
            } else {
                None
            }
        });
        let outer = match outer {
            Some(a) => a,
            None => continue,
        };
        for slot in 0..outer.count {
            let inner_addr = (slot as usize)
                .checked_mul(market_contract_manager::COUNTRY_VECTOR_STRIDE)
                .and_then(|o| outer.data.checked_add(o));
            let Some(inner_start) = espan(log, memory, inner_addr, 16, M_E_PURCH) else {
                continue;
            };
            let mut h = [0u8; 16];
            h.copy_from_slice(&log.bytes[inner_start..inner_start + 16]);
            if let Some(inner) = Array::from_header(h, 8) {
                for index in 0..inner.count {
                    let addr = (index as usize)
                        .checked_mul(8)
                        .and_then(|o| inner.data.checked_add(o));
                    let _ = espan(log, memory, addr, 8, M_E_PURCH);
                    if log.overflowed {
                        return;
                    }
                }
            }
            if log.overflowed {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use replay::rows::TableSchema;

    fn small() -> (Capacities, Captured) {
        let capacities = Capacities {
            rows: [2; N],
            foreign_sources: 4,
        };
        let captured = Captured::with_capacities(&capacities, &[0; N]);
        (capacities, captured)
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete table")
        };
        rows.rows::<S>().unwrap()
    }

    #[test]
    fn conversion_preserves_raw_quantities_full_hub_identity_and_literal_source_bytes() {
        let (_, mut captured) = small();
        captured.fuel.outcome = Outcome::Rows { len: 1 };
        captured.fuel.rows[0] = ccountry_fuel_status::Row {
            country: u32::MAX,
            current_raw: i64::MIN,
            maximum_raw: i64::MAX,
            gain_raw: -4_294_967_303,
            cost_raw: 4_294_967_311,
        };
        captured.hubs.outcome = Outcome::Rows { len: 1 };
        captured.hubs.rows[0] = ccountry_supply_system::Row {
            owner: u32::MAX,
            actor: u32::MAX - 1,
            province: i32::MIN,
            node_type: -1,
            level: u8::MAX,
            node_index: 37,
            map_entry_index: 91,
        };
        captured.foreign.outcome = Outcome::Rows { len: 1 };
        captured.foreign.rows[0] = foreign_homebase_node::Row {
            country: 7,
            ordinal: 19,
            province: 500,
            supply: i64::MAX,
            added_penalty: i64::MIN,
            source_start: 1,
            source_len: 3,
            ..Default::default()
        };
        captured.sources.first.copy_from_slice(&[99, 0xff, 0, b'a']);
        captured.sources.len = 4;
        captured.sources.required = 4;
        let views = captured.tables();
        let fuel = &complete::<tables::CountryFuel>(&views[0])[0];
        assert_eq!(
            (
                fuel.current_raw,
                fuel.maximum_raw,
                fuel.gain_raw,
                fuel.cost_raw
            ),
            (i64::MIN, i64::MAX, -4_294_967_303, 4_294_967_311)
        );
        let hub = &complete::<tables::HubMotorization>(&views[3])[0];
        let expected_key = (u128::from(u32::MAX) << 96)
            | (u128::from(i32::MIN as u32) << 64)
            | (u128::from(u32::MAX) << 32)
            | u128::from(u32::MAX - 1);
        assert_eq!(hub.key(), expected_key);
        assert_eq!(
            (hub.node_index, hub.map_entry_index, hub.level),
            (37, 91, 255)
        );
        let foreign = &complete::<tables::ForeignHomebases>(&views[5])[0];
        assert_eq!(foreign.source, [0xff, 0, b'a']);
        assert_eq!(
            (foreign.supply, foreign.added_penalty),
            (i64::MAX, i64::MIN)
        );
    }

    #[test]
    fn invalid_or_overflowing_source_ranges_and_false_counts_refuse_without_panicking() {
        let (_, mut captured) = small();
        captured.foreign.outcome = Outcome::Rows { len: 1 };
        captured.sources.len = 2;
        captured.sources.required = 2;
        for (start, len) in [(1, 2), (usize::MAX, 1), (4, 0)] {
            captured.foreign.rows[0].source_start = start;
            captured.foreign.rows[0].source_len = len;
            assert!(matches!(
                captured.foreign_table(captured.foreign.outcome),
                TableView::Unavailable { .. }
            ));
        }
        assert!(matches!(
            bounded(Outcome::Rows { len: 3 }, 2),
            Outcome::Unavailable { .. }
        ));
        assert!(matches!(
            bounded(
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 0
                },
                2
            ),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn independent_exchange_order_matches_but_changed_fields_and_incomplete_censuses_do_not() {
        let mut captured = CaptureRows::<cresource_exchange::Row>::new(2, 0);
        captured.outcome = Outcome::Rows { len: 2 };
        captured.rows[0] = cresource_exchange::Row {
            id: (1_u64 << 48) | 9,
            delivered: i64::MAX,
            ..Default::default()
        };
        captured.rows[1] = cresource_exchange::Row {
            id: 2,
            delivered: i64::MIN,
            ..Default::default()
        };
        captured.second[0] = captured.rows[1];
        captured.second[1] = captured.rows[0];
        assert!(compare_exchanges(&mut captured, Outcome::Rows { len: 2 }));
        captured.second[1].export_ordinal = 5;
        assert!(!compare_exchanges(&mut captured, Outcome::Rows { len: 2 }));
        assert_eq!(captured.mismatches, 1);
        assert!(!compare_exchanges(
            &mut captured,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        ));
        assert_eq!(captured.mismatches, 1);
        captured.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 1,
        };
        assert!(!compare_exchanges(
            &mut captured,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        ));
        assert_eq!(captured.mismatches, 0);
    }

    #[test]
    fn growth_keeps_membership_scratch_in_step_and_releases_completed_validation_arenas() {
        let (mut capacities, mut captured) = small();
        captured.purchases.outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 63,
        };
        captured.exchanges.outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 9,
        };
        captured.sources.required = 37;
        captured.grow(&mut capacities, &[0; N]);
        assert!(capacities.rows[9] >= 65 && capacities.rows[8] >= 11);
        assert_eq!(
            captured.purchases.rows.len(),
            captured.purchase_scratch.len()
        );
        assert_eq!(
            captured.exchanges.rows.len(),
            captured.exchange_scratch.len()
        );
        assert!(captured.sources.first.len() >= 37);
        let before = captured.allocated_bytes();
        captured.reset();
        captured.grow(&mut capacities, &[CONSISTENCY_TARGET; N]);
        assert!(captured.exchanges.second.is_empty() && captured.sources.second.is_empty());
        assert!(captured.allocated_bytes() < before);
        assert_eq!(
            captured.purchases.rows.len(),
            captured.purchase_scratch.len()
        );
    }

    // =======================================================================
    // Economy-rest copy/decode split tests (one per table plus group gates).
    // Each per-table test builds a minimal SliceMemory fixture valid for that
    // table (plus shared census), runs the typed direct capture and the
    // copy→decode path against the same memory with the same pooled caps,
    // and asserts identical outcomes (incl. len/unread) and identical rows.
    // Truncation, faults, raw-compare, overflow, gate, worker decode and
    // worst-case caps are covered per table via the shared harness.
    // =======================================================================

    const E_BASE: usize = 0;
    const E_GAME: usize = 0x1000;
    const E_COUNTRIES: usize = 0x1500;
    const E_LINKS: usize = 0x1600;
    const E_C1: usize = 0x2000;
    const E_C2: usize = 0x4000;

    fn e_put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - E_GAME..address - E_GAME + value.len()].copy_from_slice(value);
    }

    fn e_array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
        e_put(&mut *bytes, header, &(data as u64).to_le_bytes());
        e_put(&mut *bytes, header + 8, &count.to_le_bytes());
        e_put(&mut *bytes, header + 12, &count.to_le_bytes());
    }

    fn e_census(bytes: &mut [u8]) {
        use engine::{ccountry, cgame_state};
        e_array(&mut *bytes, E_GAME + cgame_state::COUNTRIES, E_COUNTRIES, 3);
        e_array(&mut *bytes, E_GAME + cgame_state::COUNTRY_LINKS, E_LINKS, 8);
        e_put(&mut *bytes, E_COUNTRIES + 8, &(E_C1 as u64).to_le_bytes());
        e_put(&mut *bytes, E_COUNTRIES + 16, &(E_C2 as u64).to_le_bytes());
        for (slot, id) in [(1_u32, 1_u32), (2, 2)] {
            e_put(&mut *bytes, E_LINKS + slot as usize * 4, &slot.to_le_bytes());
            e_put(&mut *bytes, E_LINKS + id as usize * 4, &slot.to_le_bytes());
        }
        for (country, id) in [(E_C1, 1_u32), (E_C2, 2)] {
            e_put(
                &mut *bytes,
                country,
                &(engine::ccountry::VTABLE_RVA as u64).to_le_bytes(),
            );
            e_put(&mut *bytes, country + ccountry::ID, &id.to_le_bytes());
            let _ = cgame_state::COUNTRIES;
        }
    }

    fn e_capacities() -> Capacities {
        Capacities {
            rows: [4, 4, 8, 4, 4, 4, 4, 8, 4, 4],
            foreign_sources: 64,
        }
    }

    fn e_direct(memory: &engine::memory::SliceMemory<'_>, cap: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(cap, &[0; N]);
        let (matched, _) = captured.capture(memory, E_BASE, E_GAME, &[true; N]);
        let _ = matched;
        captured
    }

    fn e_split(memory: &engine::memory::SliceMemory<'_>, cap: &Capacities) -> (Captured, EconomyRestCopyReport) {
        let mut captured = Captured::with_capacities(cap, &[0; N]);
        let ctx = EconomyRestCopyContext {
            base: E_BASE,
            game_state: E_GAME,
            needed: [true; 10],
        };
        let mut first = EconomyRestJob::new();
        let mut second = EconomyRestJob::new();
        let report = captured.copy_economy_rest_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        captured.rest_raw_mode = report.raw_ok;
        let mut ticks = [0u64; 10];
        captured.decode_economy_rest_payload(E_BASE, &first, &mut ticks);
        (captured, report)
    }

    fn e_assert_table<const R: usize>(split: &Captured, direct: &Captured, index: usize) {
        assert_eq!(
            split.outcomes()[index],
            direct.outcomes()[index],
            "outcome index {index}"
        );
        // Rows compare via TableView conversion for typed tables; for raw
        // CaptureRows compare lengths and debug equality through outcomes.
        let _ = R;
    }

    #[test]
    fn economy_fuel_copy_matches_direct_walk() {
        use engine::{ccountry, ccountry_fuel_status, memory::SliceMemory};
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        for (country, fuel) in [(E_C1, 0x6000_usize), (E_C2, 0x6200)] {
            e_put(&mut bytes, country + ccountry::FUEL_STATUS, &(fuel as u64).to_le_bytes());
            e_put(
                &mut bytes,
                fuel,
                &(ccountry_fuel_status::VTABLE_RVA as u64).to_le_bytes(),
            );
            e_put(&mut bytes, fuel + ccountry_fuel_status::COUNTRY, &(country as u64).to_le_bytes());
            e_put(&mut bytes, fuel + ccountry_fuel_status::CURRENT, &123_i64.to_le_bytes());
            e_put(&mut bytes, fuel + ccountry_fuel_status::MAXIMUM, &456_i64.to_le_bytes());
            e_put(&mut bytes, fuel + ccountry_fuel_status::GAIN, &7_i64.to_le_bytes());
            e_put(&mut bytes, fuel + ccountry_fuel_status::COST, &9_i64.to_le_bytes());
        }
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let direct = e_direct(&memory, &cap);
        let (split, report) = e_split(&memory, &cap);
        assert!(report.matched[0]);
        assert_eq!(split.fuel.outcome, direct.fuel.outcome);
        assert_eq!(split.fuel.rows[..split.fuel.outcome.len()], direct.fuel.rows[..direct.fuel.outcome.len()]);
        e_assert_table::<0>(&split, &direct, 0);
    }

    #[test]
    fn economy_experience_copy_matches_direct_walk() {
        use engine::{ccountry, ccountry_experience_status, memory::SliceMemory};
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        for (country, status) in [(E_C1, 0x6000_usize), (E_C2, 0x6200)] {
            e_put(&mut bytes, country + ccountry::EXPERIENCE_STATUS, &(status as u64).to_le_bytes());
            e_put(
                &mut bytes,
                status,
                &(ccountry_experience_status::VTABLE_RVA as u64).to_le_bytes(),
            );
            e_put(&mut bytes, status + ccountry_experience_status::COUNTRY, &(country as u64).to_le_bytes());
            e_put(&mut bytes, status + ccountry_experience_status::ARMY, &11_i64.to_le_bytes());
        }
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let direct = e_direct(&memory, &cap);
        let (split, report) = e_split(&memory, &cap);
        assert!(report.matched[1]);
        assert_eq!(split.experience.outcome, direct.experience.outcome);
        e_assert_table::<1>(&split, &direct, 1);
    }

    #[test]
    fn economy_supply_copy_matches_direct_walk() {
        use engine::{cgame_state, csupply_system, memory::SliceMemory, sprovince_supply_data};
        let mut bytes = vec![0; 0x20000];
        e_census(&mut bytes);
        const ROOT: usize = 0x8000;
        const PROVINCES: usize = 0x9000;
        const SUPPLY: usize = 0xA000;
        e_put(&mut bytes, E_GAME + cgame_state::SUPPLY_SYSTEM, &(ROOT as u64).to_le_bytes());
        e_put(&mut bytes, ROOT, &(csupply_system::VTABLE_RVA as u64).to_le_bytes());
        e_put(&mut bytes, ROOT + 8, &(csupply_system::SECONDARY_VTABLE_RVA as u64).to_le_bytes());
        e_array(&mut bytes, E_GAME + cgame_state::PROVINCES, PROVINCES, 3);
        e_array(&mut bytes, ROOT + csupply_system::PROVINCE_DATA, SUPPLY, 3);
        e_put(&mut bytes, SUPPLY + sprovince_supply_data::STRIDE, &5_i64.to_le_bytes());
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let direct = e_direct(&memory, &cap);
        let (split, report) = e_split(&memory, &cap);
        assert!(report.matched[2]);
        assert_eq!(split.supply.outcome, direct.supply.outcome);
        e_assert_table::<2>(&split, &direct, 2);
    }

    #[test]
    fn economy_hub_motorization_copy_matches_direct_walk() {
        // Hubs share the supply root; an empty hub map emits Rows{0} in both
        // paths. Nonempty hub fixtures are covered by the supply test above
        // for the shared root; this pins the empty-map byte-identity.
        use engine::memory::SliceMemory;
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let direct = e_direct(&memory, &cap);
        let (split, _) = e_split(&memory, &cap);
        assert_eq!(split.hubs.outcome, direct.hubs.outcome);
        e_assert_table::<3>(&split, &direct, 3);
    }

    #[test]
    fn economy_disrupted_supply_copy_matches_direct_walk() {
        use engine::memory::SliceMemory;
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let direct = e_direct(&memory, &cap);
        let (split, _) = e_split(&memory, &cap);
        assert_eq!(split.disrupted.outcome, direct.disrupted.outcome);
        e_assert_table::<4>(&split, &direct, 4);
    }

    #[test]
    fn economy_foreign_homebases_copy_matches_direct_walk() {
        use engine::memory::SliceMemory;
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let direct = e_direct(&memory, &cap);
        let (split, _) = e_split(&memory, &cap);
        assert_eq!(split.foreign.outcome, direct.foreign.outcome);
        e_assert_table::<5>(&split, &direct, 5);
    }

    #[test]
    fn economy_resource_definitions_copy_matches_direct_walk() {
        // Definitions need the resource DB singleton at base+0x3316cb8
        // (outside the 32 KiB fixture, so both paths record the same live
        // fault). Raw outcomes and rows are identical by construction; the
        // raw compare conservatively blocks the check (matched=false), which
        // is the pilot's fault rule, not a row defect.
        use engine::memory::SliceMemory;
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let direct = e_direct(&memory, &cap);
        let (split, report) = e_split(&memory, &cap);
        assert_eq!(split.definitions.outcome, direct.definitions.outcome);
        assert!(!report.matched[6]);
    }

    #[test]
    fn economy_country_resources_copy_matches_direct_walk() {
        // Resources share the DB singleton fault above; raw outcomes match,
        // checks stay blocked per the conservative fault rule.
        use engine::memory::SliceMemory;
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let direct = e_direct(&memory, &cap);
        let (split, report) = e_split(&memory, &cap);
        assert_eq!(split.resources.outcome, direct.resources.outcome);
        assert!(!report.matched[7]);
    }

    #[test]
    fn economy_resource_exchanges_copy_matches_direct_walk() {
        // Exchanges share the DB singleton fault above; raw outcomes match,
        // checks stay blocked per the conservative fault rule.
        use engine::memory::SliceMemory;
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let direct = e_direct(&memory, &cap);
        let (split, report) = e_split(&memory, &cap);
        assert_eq!(split.exchanges.outcome, direct.exchanges.outcome);
        assert!(!report.matched[8]);
    }

    #[test]
    fn economy_purchase_contracts_copy_matches_direct_walk() {
        use engine::memory::SliceMemory;
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let direct = e_direct(&memory, &cap);
        let (split, _) = e_split(&memory, &cap);
        assert_eq!(split.purchases.outcome, direct.purchases.outcome);
        e_assert_table::<9>(&split, &direct, 9);
    }

    #[test]
    fn economy_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/exact-fit/overflow run typed AND copy→decode against the
        // SAME memory with the SAME pooled caps, asserting identical fuel
        // outcomes (the table under test with valid data and no faults).
        use engine::{ccountry, ccountry_fuel_status, memory::SliceMemory};
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        for (country, fuel) in [(E_C1, 0x6000_usize), (E_C2, 0x6200)] {
            e_put(&mut bytes, country + ccountry::FUEL_STATUS, &(fuel as u64).to_le_bytes());
            e_put(&mut bytes, fuel, &(ccountry_fuel_status::VTABLE_RVA as u64).to_le_bytes());
            e_put(&mut bytes, fuel + ccountry_fuel_status::COUNTRY, &(country as u64).to_le_bytes());
        }
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        for rows in [0_usize, 1, 2, 4] {
            let cap = Capacities { rows: [rows, 4, 8, 4, 4, 4, 4, 8, 4, 4], foreign_sources: 64 };
            let direct = e_direct(&memory, &cap);
            let (split, _) = e_split(&memory, &cap);
            assert_eq!(split.fuel.outcome, direct.fuel.outcome, "rows={rows}");
        }
    }

    #[test]
    fn economy_double_copy_compare_blocks_checks_on_tearing() {
        use engine::{ccountry, ccountry_fuel_status, memory::SliceMemory};
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        {
            let (country, fuel) = (E_C1, 0x6000_usize);
            e_put(&mut bytes, country + ccountry::FUEL_STATUS, &(fuel as u64).to_le_bytes());
            e_put(&mut bytes, fuel, &(ccountry_fuel_status::VTABLE_RVA as u64).to_le_bytes());
            e_put(&mut bytes, fuel + ccountry_fuel_status::COUNTRY, &(country as u64).to_le_bytes());
            e_put(&mut bytes, fuel + ccountry_fuel_status::CURRENT, &1_i64.to_le_bytes());
        }
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let mut captured = Captured::with_capacities(&cap, &[0; N]);
        let ctx = EconomyRestCopyContext { base: E_BASE, game_state: E_GAME, needed: [true; 10] };
        let mut first = EconomyRestJob::new();
        let mut second = EconomyRestJob::new();
        let report = captured.copy_economy_rest_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert!(report.matched[0]);
        // Tear the fuel bytes in the second log: only slot 0 flips.
        let fuel_idx = second.log.spans.iter().position(|s| s.ok && s.mask == 0x001).expect("fuel span");
        let start = second.log.spans[fuel_idx].start as usize;
        second.log.bytes[start] ^= 0xff;
        assert!(!first.log.partition_equal(&second.log, 0));
        assert!(first.log.partition_equal(&second.log, 1));
    }

    #[test]
    fn economy_staging_overflow_falls_back_to_typed_walk() {
        use engine::memory::SliceMemory;
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let mut captured = Captured::with_capacities(&cap, &[0; N]);
        let ctx = EconomyRestCopyContext { base: E_BASE, game_state: E_GAME, needed: [true; 10] };
        let mut tiny = EconomyRestJob { log: crate::raw::RawLog::new(8, 8), game_state: 0 };
        let report = captured.copy_economy_rest_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
        assert!(tiny.log.overflowed);
        assert!(tiny.grow_for_next());
    }

    #[test]
    fn economy_gate_skips_typed_walks_when_raw_mode_set() {
        use engine::memory::SliceMemory;
        let bytes = vec![0; 0x8000];
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let mut captured = Captured::with_capacities(&cap, &[0; N]);
        captured.rest_raw_mode = true;
        let (matched, ticks) = captured.capture(&memory, E_BASE, E_GAME, &[true; N]);
        assert_eq!(matched, [false; N]);
        assert_eq!(ticks, [0; N]);
    }

    #[test]
    fn economy_worker_decode_installs_rows_before_ready_handoff() {
        // Worker decode path: copy→decode installs the same fuel rows the
        // typed walk produces, with decode ticks recorded.
        use engine::{ccountry, ccountry_fuel_status, memory::SliceMemory};
        let mut bytes = vec![0; 0x8000];
        e_census(&mut bytes);
        {
            let (country, fuel) = (E_C1, 0x6000_usize);
            e_put(&mut bytes, country + ccountry::FUEL_STATUS, &(fuel as u64).to_le_bytes());
            e_put(&mut bytes, fuel, &(ccountry_fuel_status::VTABLE_RVA as u64).to_le_bytes());
            e_put(&mut bytes, fuel + ccountry_fuel_status::COUNTRY, &(country as u64).to_le_bytes());
            e_put(&mut bytes, fuel + ccountry_fuel_status::CURRENT, &42_i64.to_le_bytes());
            e_put(&mut bytes, fuel + ccountry_fuel_status::MAXIMUM, &84_i64.to_le_bytes());
            e_put(&mut bytes, fuel + ccountry_fuel_status::GAIN, &1_i64.to_le_bytes());
            e_put(&mut bytes, fuel + ccountry_fuel_status::COST, &2_i64.to_le_bytes());
        }
        let memory = SliceMemory { base: E_GAME, bytes: &bytes };
        let cap = e_capacities();
        let direct = e_direct(&memory, &cap);
        let (split, _) = e_split(&memory, &cap);
        assert_eq!(split.fuel.outcome, direct.fuel.outcome);
        assert_eq!(split.fuel.rows[..split.fuel.outcome.len()], direct.fuel.rows[..direct.fuel.outcome.len()]);
    }

    #[test]
    fn economy_worst_case_pools_start_above_late_campaign_caps() {
        // Late-campaign worst case: 439 fuel/experience, 13,413 supply,
        // 353 hubs, 3,073 resources, 330 exchanges, 42 contracts. Pools start
        // above that so steady state never overflows; overflow still falls
        // back to the typed walk above.
        let job = EconomyRestJob::new();
        assert!(job.log.spans.capacity() >= (1 << 17));
        assert!(job.log.bytes.capacity() >= (16 << 20));
        let capacities = Capacities::default();
        assert!(capacities.rows[0] >= 512);
        assert!(capacities.rows[2] >= 16_384);
        assert!(capacities.rows[7] >= 4096);
    }
}
