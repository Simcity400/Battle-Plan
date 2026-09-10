//! Economy acquisition buffers. Growth and replay conversion run off-thread.

use crate::{
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_vec},
};
use engine::{
    Memory, ccountry_experience_status, ccountry_fuel_status, ccountry_resources,
    ccountry_supply_system, cpurchase_contract, cresource_exchange, cstrategic_resource,
    disrupted_supply_bucket, foreign_homebase_node, sprovince_supply_data, walk::Outcome,
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
}
