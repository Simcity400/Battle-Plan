//! The resources map mode's inputs and every trade-route line
//! ([`engine::resource_map`]): ResourceOrigins, ResourceRights, CountryOilFactors,
//! StateTemporaryResources and ConvoyClients are read every hour, as the game refreshes
//! origins and convoy efficiency in its hourly trade task. TradeRoutes is the sliced
//! `trade_routes` family (owner: the country holding each route): each hour reads its owner
//! slice, an owner new to the census or whose set of referenced routes changed, and every
//! owner one of whose routes a hooked route writer touched since the previous hour
//! ([`crate::raid_changes`]: the route update, which is the only caller of both path
//! builders, route construction, country resources construction and the save loader). An
//! hour whose take of the marks does not certify the previous hour's read reads every
//! owner. Every owner read and not marked is compared with its previous read
//! ([`SliceSelection::compare`]); a difference disables the family's marks for the
//! attachment. Read in the RaidState job, after the raid state inputs.
use crate::{
    clock::qpc,
    slicing::{SliceFingerprints, SliceSelection},
    table_conversion::{table, unavailable},
};
use engine::{
    Memory, ccountry, cstrategic_resource,
    resource_map::{self as native, RouteKey},
    walk::{Buffer, Outcome},
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};

pub const N: usize = 7;
pub const TABLES: [TableId; N] = [
    TableId::ResourceOrigins,
    TableId::ResourceRights,
    TableId::CountryOilFactors,
    TableId::StateTemporaryResources,
    TableId::ConvoyClients,
    TableId::TradeRoutes,
    TableId::RegionalConvoyEfficiency,
];
/// One walk reads the origins, rights, oil factors and convoy clients; the temporary
/// resources and the routes are their own reads.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    (
        "resource origins, rights, oil factors and convoy clients",
        0,
        &[0, 1, 2, 4],
    ),
    ("state temporary resources", 3, &[3]),
    ("trade routes", 5, &[5]),
    ("regional convoy efficiency of bad naval routes", 6, &[6]),
];

/// Every buffer grows on the worker from what the walk finds.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities;

/// The family's owner store, loaned for one capture, and the hook marks that join each
/// hour's read.
pub struct Stores<'a> {
    pub routes: &'a mut SliceFingerprints,
    pub changes: &'a dyn crate::raid_changes::Changes,
    /// Read every owner every hour while still comparing each unmarked one.
    pub verify: bool,
}

pub struct Captured {
    origins: Vec<native::OriginRow>,
    rights: Vec<native::RightsRow>,
    oil: Vec<native::OilRow>,
    clients: Vec<native::ClientRow>,
    temporary: Vec<native::TemporaryRow>,
    regional: Vec<native::RegionalRow>,
    pairs: Vec<(u32, u32)>,
    descriptors: Vec<cstrategic_resource::Row>,
    /// Every route a client referenced this hour, in row key order.
    census: Vec<(RouteKey, usize, bool)>,
    routes: Vec<native::RouteRow>,
    path: Vec<u32>,
    hashes: Vec<(u32, u64)>,
    outcomes: [Outcome; N],
    /// The countries whose TradeRoutes rows this hour's read replaced.
    selection: SliceSelection,
}

/// The row key order of TradeRoutes: country, class, other.
fn route_order(key: &RouteKey) -> (u32, u8, u32) {
    (key.country, key.class, key.other)
}

impl Captured {
    pub fn vacant() -> Self {
        Self::new(false)
    }
    pub fn with_capacities(_capacity: &Capacities) -> Self {
        Self::new(true)
    }
    fn new(storage: bool) -> Self {
        Self {
            origins: Vec::new(),
            rights: Vec::new(),
            oil: Vec::new(),
            clients: Vec::new(),
            temporary: Vec::new(),
            regional: Vec::new(),
            pairs: Vec::new(),
            descriptors: Vec::new(),
            census: Vec::new(),
            routes: Vec::new(),
            path: Vec::new(),
            hashes: Vec::new(),
            outcomes: [crate::MISSING; N],
            selection: SliceSelection::new(storage),
        }
    }
    pub fn reset(&mut self) {
        self.outcomes = [crate::MISSING; N];
    }
    pub fn fail_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        self.outcomes = TABLES.map(|table| crate::failed_capture(table, hour, force_full, reason));
        self.selection.reset(hour, force_full);
    }
    pub fn allocated_bytes(&self) -> usize {
        use std::mem::size_of;
        self.origins.capacity() * size_of::<native::OriginRow>()
            + self.rights.capacity() * size_of::<native::RightsRow>()
            + self.oil.capacity() * size_of::<native::OilRow>()
            + self.clients.capacity() * size_of::<native::ClientRow>()
            + self.temporary.capacity() * size_of::<native::TemporaryRow>()
            + self.regional.capacity() * size_of::<native::RegionalRow>()
            + self.pairs.capacity() * size_of::<(u32, u32)>()
            + self.descriptors.capacity() * size_of::<cstrategic_resource::Row>()
            + self.census.capacity() * size_of::<(RouteKey, usize, bool)>()
            + self.routes.capacity() * size_of::<native::RouteRow>()
            + self.path.capacity() * size_of::<u32>()
            + self.hashes.capacity() * size_of::<(u32, u64)>()
    }
    /// Buffers grow on the worker during the read.
    pub fn grow(&mut self, _capacity: &mut Capacities) {}

    pub fn reset_selection(&mut self, hour: i32, force_full: bool) {
        self.selection.reset(hour, force_full);
    }

    /// The countries whose TradeRoutes rows this frame's read replaced.
    pub fn route_selection(&self) -> &SliceSelection {
        &self.selection
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        self.outcomes
    }

    /// Fixture read without the family's owner store: every route is read.
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_growing(memory, base, game_state, hour, force_full, None)
    }

    /// Worker-only: every buffer grows from what the walks find.
    pub fn capture_growing(
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
        self.selection.reset(hour, full);
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
            self.outcomes[1] = sorted(&mut self.rights, outcomes.rights, |row| {
                (row.state, row.resource)
            });
            self.outcomes[2] = sorted(&mut self.oil, outcomes.oil, |row| row.country);
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
                Outcome::Rows { len } => {
                    let outcome = native::walk_temporary(
                        memory,
                        base,
                        game_state,
                        &self.descriptors[..len],
                        Buffer::Growing(&mut self.temporary),
                    );
                    sorted(&mut self.temporary, outcome, |row| {
                        (row.state, row.resource)
                    })
                }
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
                    let outcome = native::walk_regional(
                        memory,
                        base,
                        game_state,
                        &self.origins[..origins],
                        &self.clients[..clients],
                        &mut self.pairs,
                        Buffer::Growing(&mut self.regional),
                    );
                    sorted(&mut self.regional, outcome, |row| (row.country, row.region))
                }
                _ => Outcome::Unavailable {
                    reason: "regional convoy efficiency needs complete origins and convoy clients",
                },
            };
        }
        ticks[6] = elapsed(begin);
        ticks
    }

    /// The route census, its owners' selection and the selected owners' routes.
    fn routes(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        mut stores: Option<Stores<'_>>,
    ) -> Outcome {
        let census = self.census();
        if let Some(stores) = stores.as_mut() {
            let reader = ccountry::Reader::new(memory, base, game_state).ok();
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
                engine::allocation::reserve(&mut self.routes, 1)
                    .map_err(|_| "trade route allocation failed")?;
                self.routes.push(row);
            }
            Ok(Outcome::Rows {
                len: self.routes.len(),
            })
        });
        let outcome = outcome.unwrap_or_else(|reason| Outcome::Unavailable { reason });
        if let Some(stores) = stores {
            self.compare(stores, outcome, reader_ids(memory, base, game_state));
        }
        outcome
    }

    /// Scan the census's owners into the family's store, then mark every owner one of whose
    /// routes a hooked writer touched; `resolve` names a mark's raw link's country, and a
    /// mark it cannot name certifies nothing.
    fn select(
        &mut self,
        stores: &mut Stores<'_>,
        complete: bool,
        resolve: &dyn Fn(u32) -> Option<u32>,
    ) {
        use crate::raid_changes::Family;
        // Each owner's print is its set of referenced routes and where they live, so a
        // route newly referenced, dropped or rebuilt elsewhere reads its owner.
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
                .scan(stores.routes, owner, Some(print.finish()));
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
        selection.certify(stores.routes, epoch.filter(|_| resolved), stores.verify);
    }

    /// Every route a client referenced this hour, in TradeRoutes' key order. A key naming
    /// two routes refuses the census.
    fn census(&mut self) -> Result<(), &'static str> {
        let (Outcome::Rows { len: origins }, Outcome::Rows { len: clients }) =
            (self.outcomes[0], self.outcomes[4])
        else {
            return Err("trade route census needs complete origins and convoy clients");
        };
        self.census.clear();
        engine::allocation::reserve(&mut self.census, origins + clients)
            .map_err(|_| "trade route census allocation failed")?;
        // Own and rights-holder origins alike: each carries its route to its list's owner.
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

    /// Compare every owner this hour read and no hook marked with its previous read,
    /// disable the family's marks at a difference, and record the hour's counts.
    fn compare(&mut self, stores: Stores<'_>, outcome: Outcome, ids: Option<usize>) {
        use crate::raid_changes::{Counts, Family, record};
        let complete = matches!(outcome, Outcome::Rows { .. }) && self.route_hashes();
        let (verified, mismatches) = self
            .selection
            .compare(stores.routes, complete.then_some(&self.hashes[..]));
        if mismatches != 0 {
            stores.changes.mismatch(Family::Routes);
        }
        record(
            Family::Routes,
            &Counts {
                marked: u64::from(self.selection.marked_count),
                read: self.selection.read_owners().count() as u64,
                early: u64::from(self.selection.marked_early()),
                full: self.selection.force_full,
                verified,
                mismatches,
            },
            ids,
        );
    }

    /// Each read owner's content: every field of its routes. False when a path's span or
    /// the storage fails.
    fn route_hashes(&mut self) -> bool {
        self.hashes.clear();
        if engine::allocation::reserve(&mut self.hashes, self.routes.len()).is_err() {
            return false;
        }
        for route in &self.routes {
            let Some(path) = route
                .path_start
                .checked_add(route.path_len)
                .and_then(|end| self.path.get(route.path_start..end))
            else {
                return false;
            };
            let mut hash = crate::slicing::Digest::new();
            hash.u64(u64::from(route.key.class))
                .u64(u64::from(route.key.other))
                .u64(u64::from(route.kind))
                .u64(u64::from(route.from))
                .u64(u64::from(route.to))
                .u64(route.source_state.map(|id| u64::from(id) + 1).unwrap_or(0))
                .u64(
                    route
                        .destination_state
                        .map(|id| u64::from(id) + 1)
                        .unwrap_or(0),
                )
                .u64(path.len() as u64);
            for &id in path {
                hash.u64(u64::from(id));
            }
            self.hashes.push((route.key.country, hash.finish()));
        }
        // One content hash per owner, in its rows' order.
        self.hashes.dedup_by(|next, kept| {
            next.0 == kept.0 && {
                kept.1 = crate::slicing::Digest::new()
                    .u64(kept.1)
                    .u64(next.1)
                    .finish();
                true
            }
        });
        true
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
                        path: engine::allocation::owned_vec(engine::allocation::copy_vec(
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
        ]
    }
}

/// Raw country link ids the country table can hold, sizing the route mark set.
fn reader_ids(memory: &impl Memory, base: usize, game_state: usize) -> Option<usize> {
    ccountry::Reader::new(memory, base, game_state)
        .ok()
        .map(|reader| reader.arrays()[1].count as usize)
}

/// Sort a complete read's rows by their table key; two rows with one key refuse it.
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
