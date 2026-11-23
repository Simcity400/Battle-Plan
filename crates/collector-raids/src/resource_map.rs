use crate::{
    clock::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    table_conversion::{table, unavailable},
};
use engine_native_memory::{Memory, walk::{Buffer, Outcome}};
use engine_native::ccountry;
use engine_resources::{cstrategic_resource, resource_map as native, resource_map::RouteKey};
use replay_data::{rows, rows::TableId, rows::tables};
use replay_frame::frame::TableView;

pub const N: usize = 8;
pub const TABLES: [TableId; N] = [
    TableId::ResourceOrigins,
    TableId::ResourceRights,
    TableId::CountryOilFactors,
    TableId::StateTemporaryResources,
    TableId::ConvoyClients,
    TableId::TradeRoutes,
    TableId::RegionalConvoyEfficiency,
    TableId::StrategicRegionWeights,
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities;

collector_primitives::capacity_max!(Capacities;);

pub struct Stores<'a> {
    pub routes: &'a mut SliceFingerprints,
    pub changes: &'a dyn crate::raid_changes::Changes,
}

pub struct Captured {
    pending_owned: bool,
    origins: Vec<native::OriginRow>,
    rights: Vec<native::RightsRow>,
    oil: Vec<native::OilRow>,
    clients: Vec<native::ClientRow>,
    temporary: Vec<native::TemporaryRow>,
    regional: Vec<native::RegionalRow>,
    weights: Vec<native::RegionWeightRow>,
    pairs: Vec<(u32, u32)>,
    descriptors: Vec<cstrategic_resource::Row>,
    census: Vec<(RouteKey, usize, bool)>,
    routes: Vec<native::RouteRow>,
    path: Vec<u32>,
    outcomes: [Outcome; N],
    selection: SliceSelection,
}

fn route_order(key: &RouteKey) -> (u32, u8, u32) {
    (key.country, key.class, key.other)
}

impl Captured {
    pub fn vacant() -> Self {
        Self::new()
    }
    pub fn with_capacities(_capacity: &Capacities) -> Self {
        Self::new()
    }
    fn new() -> Self {
        Self {
            pending_owned: false,
            origins: Vec::new(),
            rights: Vec::new(),
            oil: Vec::new(),
            clients: Vec::new(),
            temporary: Vec::new(),
            regional: Vec::new(),
            weights: Vec::new(),
            pairs: Vec::new(),
            descriptors: Vec::new(),
            census: Vec::new(),
            routes: Vec::new(),
            path: Vec::new(),
            outcomes: [crate::MISSING; N],
            selection: SliceSelection::new(),
        }
    }
    pub fn reset(&mut self) {
        self.pending_owned = false;
        self.outcomes = [crate::MISSING; N];
    }
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.pending_owned = false;
        self.outcomes = TABLES.map(|table| crate::failed_capture(table, hour, force_full, reason));
        self.selection.reset_hooked(hour, force_full);
    }
    pub fn allocated_bytes(&self) -> usize {
        use std::mem::size_of;
        self.origins.capacity() * size_of::<native::OriginRow>()
            + self.rights.capacity() * size_of::<native::RightsRow>()
            + self.oil.capacity() * size_of::<native::OilRow>()
            + self.clients.capacity() * size_of::<native::ClientRow>()
            + self.temporary.capacity() * size_of::<native::TemporaryRow>()
            + self.regional.capacity() * size_of::<native::RegionalRow>()
            + self.weights.capacity() * size_of::<native::RegionWeightRow>()
            + self.pairs.capacity() * size_of::<(u32, u32)>()
            + self.descriptors.capacity() * size_of::<cstrategic_resource::Row>()
            + self.census.capacity() * size_of::<(RouteKey, usize, bool)>()
            + self.routes.capacity() * size_of::<native::RouteRow>()
            + self.path.capacity() * size_of::<u32>()
    }
    pub fn grow(&mut self, _capacity: &mut Capacities) {}

    pub fn reset_selection(&mut self, hour: i32, force_full: bool) {
        self.selection.reset_hooked(hour, force_full);
    }

    pub fn route_selection(&self) -> &SliceSelection {
        &self.selection
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        self.outcomes
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        stores: Option<Stores<'_>>,
    ) -> [u64; N] {
        let ticks = self.capture_deferred(memory, base, game_state, hour, force_full, stores);
        let start = qpc();
        self.settle_owned();
        let mut ticks = ticks;
        ticks[0] += elapsed(start);
        ticks
    }

    pub fn capture_deferred(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        stores: Option<Stores<'_>>,
    ) -> [u64; N] {
        self.reset();
        let full = force_full || stores.is_none();
        self.selection.reset_hooked(hour, full);
        let mut ticks = [0; N];
        let begin = qpc();
        {
            let _reads = crate::read_accounting::table(TABLES[0]);
            let outcomes = native::walk_buffer(
                memory,
                base,
                game_state,
                native::Outputs {
                    origins: Buffer::Growing(&mut self.origins),
                    rights: Buffer::Growing(&mut self.rights),
                    oil: Buffer::Growing(&mut self.oil),
                    clients: Buffer::Growing(&mut self.clients),
                },
            );
            self.outcomes[0] = sorted(&mut self.origins, outcomes.origins, |row| {
                (row.state, row.country)
            });
            self.outcomes[1] = outcomes.rights;
            self.outcomes[2] = outcomes.oil;
            self.outcomes[4] = sorted(&mut self.clients, outcomes.clients, |row| row.handle);
        }
        ticks[0] = elapsed(begin);
        let begin = qpc();
        {
            let _reads = crate::read_accounting::table(TABLES[3]);
            let descriptors = cstrategic_resource::walk_buffer(
                memory,
                base,
                Buffer::Growing(&mut self.descriptors),
            );
            self.outcomes[3] = match descriptors {
                Outcome::Rows { len } => native::walk_temporary(
                    memory,
                    base,
                    game_state,
                    &self.descriptors[..len],
                    Buffer::Growing(&mut self.temporary),
                ),
                _ => Outcome::Unavailable {
                    reason: "resource descriptor census incomplete or inconsistent",
                },
            };
        }
        ticks[3] = elapsed(begin);
        let begin = qpc();
        {
            let _reads = crate::read_accounting::table(TABLES[5]);
            self.outcomes[5] = self.routes(memory, base, game_state, stores);
        }
        ticks[5] = elapsed(begin);
        let begin = qpc();
        {
            let _reads = crate::read_accounting::table(TABLES[6]);
            self.outcomes[6] = match (self.outcomes[0], self.outcomes[4]) {
                (Outcome::Rows { len: origins }, Outcome::Rows { len: clients }) => {
                    native::walk_regional(
                        memory,
                        base,
                        game_state,
                        &self.origins[..origins],
                        &self.clients[..clients],
                        &mut self.pairs,
                        Buffer::Growing(&mut self.regional),
                    )
                }
                _ => Outcome::Unavailable {
                    reason: "regional convoy efficiency needs complete origins and convoy clients",
                },
            };
        }
        ticks[6] = elapsed(begin);
        let begin = qpc();
        self.outcomes[7] = if replay_data::cadence::should_walk(TABLES[7], hour, force_full) {
            let _reads = crate::read_accounting::table(TABLES[7]);
            native::walk_region_weights(
                memory,
                base,
                game_state,
                Buffer::Growing(&mut self.weights),
            )
        } else {
            Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
            }
        };
        ticks[7] = elapsed(begin);
        self.pending_owned = true;
        ticks
    }

    pub fn settle_owned(&mut self) {
        if !self.pending_owned {
            return;
        }
        self.outcomes[1] = sorted(&mut self.rights, self.outcomes[1], |row| {
            (row.state, row.resource)
        });
        self.outcomes[2] = sorted(&mut self.oil, self.outcomes[2], |row| row.country);
        self.outcomes[3] = sorted(&mut self.temporary, self.outcomes[3], |row| {
            (row.state, row.resource)
        });
        self.outcomes[6] = sorted(&mut self.regional, self.outcomes[6], |row| {
            (row.country, row.region)
        });
        self.outcomes[7] = sorted(&mut self.weights, self.outcomes[7], |row| row.region);
        self.pending_owned = false;
    }

    fn routes(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        mut stores: Option<Stores<'_>>,
    ) -> Outcome {
        let census = self.census();
        let mut ids = None;
        if let Some(stores) = stores.as_mut() {
            let reader = ccountry::Reader::new(memory, base, game_state).ok();
            ids = reader
                .as_ref()
                .map(|reader| reader.arrays()[1].count as usize);
            self.select(stores, census.is_ok(), &|raw| {
                reader
                    .as_ref()
                    .and_then(|reader| reader.resolve(raw).ok())
                    .map(|country| country.id)
            });
        }
        let outcome = census.and_then(|()| {
            self.routes.clear();
            self.path.clear();
            for &(key, address, market) in &self.census {
                if !self.selection.contains(key.country) {
                    continue;
                }
                let row = native::read_route(
                    memory,
                    base,
                    key,
                    address,
                    market.then_some(game_state),
                    &mut self.path,
                )?;
                engine_allocation::reserve(&mut self.routes, 1)
                    .map_err(|_| "trade route allocation failed")?;
                self.routes.push(row);
            }
            Ok(Outcome::Rows {
                len: self.routes.len(),
            })
        });
        let outcome = outcome.unwrap_or_else(|reason| Outcome::Unavailable { reason });
        if stores.is_some() {
            self.record_counts(ids);
        }
        outcome
    }

    fn select(
        &mut self,
        stores: &mut Stores<'_>,
        complete: bool,
        resolve: &dyn Fn(u32) -> Option<u32>,
    ) {
        use crate::raid_changes::Family;
        let mut start = 0;
        while complete && start < self.census.len() {
            let owner = self.census[start].0.country;
            let end =
                start + self.census[start..].partition_point(|(key, _, _)| key.country == owner);
            let mut print = crate::slicing::Digest::new();
            for (key, address, market) in &self.census[start..end] {
                print
                    .u64(u64::from(key.class))
                    .u64(u64::from(key.other))
                    .u64(*address as u64)
                    .u64(u64::from(*market));
            }
            self.selection
                .scan_marking(stores.routes, owner, Some(print.finish()));
            start = end;
        }
        stores.routes.finish_scan();
        self.selection.finish_pass(stores.routes, complete);
        let mut resolved = true;
        let selection = &mut self.selection;
        let epoch = stores
            .changes
            .take(Family::Routes, &mut |raw| match resolve(raw as u32) {
                Some(country) => selection.mark(country),
                None => resolved = false,
            });
        selection.certify(stores.routes, epoch.filter(|_| resolved));
    }

    fn census(&mut self) -> Result<(), &'static str> {
        let (Outcome::Rows { len: origins }, Outcome::Rows { len: clients }) =
            (self.outcomes[0], self.outcomes[4])
        else {
            return Err("trade route census needs complete origins and convoy clients");
        };
        self.census.clear();
        engine_allocation::reserve(&mut self.census, origins + clients)
            .map_err(|_| "trade route census allocation failed")?;
        for origin in &self.origins[..origins] {
            let key = RouteKey {
                class: native::ORIGIN_ROUTE_CLASS,
                country: origin.country,
                other: origin.state,
            };
            self.census.push((key, origin.route_address, false));
        }
        for client in &self.clients[..clients] {
            self.census.push((
                client.route,
                client.route_address,
                client.category == native::MARKET,
            ));
        }
        self.census
            .sort_unstable_by_key(|&(key, address, _)| (route_order(&key), address));
        self.census.dedup_by(|next, kept| {
            next.0 == kept.0 && next.1 == kept.1 && {
                kept.2 |= next.2;
                true
            }
        });
        if self.census.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err("one trade route key names two routes");
        }
        Ok(())
    }

    fn record_counts(&self, ids: Option<usize>) {
        use crate::raid_changes::{Counts, Family, record};
        record(
            Family::Routes,
            &Counts {
                marked: u64::from(self.selection.marked_count),
                read: self.selection.read_owners().count() as u64,
                carried: self.selection.carried() as u64,
                early: u64::from(self.selection.marked_early()),
                full: self.selection.force_full,
            },
            ids,
        );
    }

    pub fn tables(&self) -> [TableView; N] {
        let path = |row: &native::RouteRow| {
            row.path_start
                .checked_add(row.path_len)
                .and_then(|end| self.path.get(row.path_start..end))
        };
        let routes = self
            .routes
            .get(..self.outcomes[5].len())
            .unwrap_or_default();
        [
            table::<tables::ResourceOrigins, _>(self.outcomes[0], &self.origins, |row| {
                rows::ResourceOriginRow {
                    ordinal: row.ordinal,
                    state: row.state,
                    country: row.country,
                    list: row.list,
                    efficiency: row.efficiency,
                    route_factor: row.route_factor,
                    local_resources_factor: row.local_resources_factor,
                    bad: row.bad,
                    convoys_assigned: row.convoys[0],
                    convoys_required: row.convoys[1],
                    convoys_base: row.convoys_base.into(),
                }
            }),
            table::<tables::ResourceRights, _>(self.outcomes[1], &self.rights, |row| {
                rows::ResourceRightRow {
                    ordinal: row.ordinal,
                    state: row.state,
                    resource: row.resource,
                    holder: row.holder,
                }
            }),
            table::<tables::CountryOilFactors, _>(self.outcomes[2], &self.oil, |row| {
                rows::CountryOilFactorRow {
                    country: row.country,
                    factor: row.factor,
                }
            }),
            table::<tables::StateTemporaryResources, _>(self.outcomes[3], &self.temporary, |row| {
                rows::StateTemporaryResourceRow {
                    state: row.state,
                    resource: row.resource,
                    amount: row.amount,
                }
            }),
            table::<tables::ConvoyClients, _>(self.outcomes[4], &self.clients, |row| {
                rows::ConvoyClientRow {
                    owner_ordinal: row.owner_ordinal,
                    other_ordinal: row.other_ordinal,
                    handle: row.handle,
                    category: row.category,
                    owner: row.owner,
                    other: row.other,
                    route_class: row.route.class,
                    route_country: row.route.country,
                    route_other: row.route.other,
                    bad: row.bad,
                    convoys_assigned: row.convoys[0],
                    convoys_required: row.convoys[1],
                    efficiency_a: row.efficiency[0].into(),
                    efficiency_b: row.efficiency[1].into(),
                    convoys_base: row.convoys_base.into(),
                }
            }),
            if routes.iter().all(|row| path(row).is_some()) {
                table::<tables::TradeRoutes, _>(self.outcomes[5], &self.routes, |row| {
                    rows::TradeRouteRow {
                        country: row.key.country,
                        class: row.key.class,
                        other: row.key.other,
                        kind: row.kind,
                        from: row.from,
                        to: row.to,
                        source_state: row.source_state,
                        destination_state: row.destination_state,
                        path: engine_allocation::owned_vec(engine_allocation::copy_vec(
                            path(row).unwrap(),
                        )),
                    }
                })
            } else {
                unavailable("trade route path outside captured storage")
            },
            table::<tables::RegionalConvoyEfficiency, _>(self.outcomes[6], &self.regional, |row| {
                rows::RegionalConvoyEfficiencyRow {
                    country: row.country,
                    region: row.region,
                    efficiency: row.efficiency,
                }
            }),
            table::<tables::StrategicRegionWeights, _>(self.outcomes[7], &self.weights, |row| {
                rows::StrategicRegionWeightRow {
                    region: row.region,
                    weight: row.weight,
                }
            }),
        ]
    }
}

fn sorted<T, K: Ord>(rows: &mut [T], outcome: Outcome, key: impl Fn(&T) -> K) -> Outcome {
    let Some(read) = rows.get_mut(..outcome.len()) else {
        return Outcome::Unavailable {
            reason: "walker row count exceeds allocated buffer",
        };
    };
    read.sort_unstable_by_key(&key);
    if read.windows(2).any(|pair| key(&pair[0]) == key(&pair[1])) {
        return Outcome::Unavailable {
            reason: "two resources map rows share one key",
        };
    }
    outcome
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

#[cfg(test)]
mod tests;
