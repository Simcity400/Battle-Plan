//! Resource balance's emit: each tag's pools and deals as runs on the family's series
//! publication, observed as the row census observed them ([`super::row`]).
//!
//! Each resource's pools of a tag run while they stay unchanged over consecutive hours of the
//! same identity: its POOL key holds the run's first hour, with the pools as an event there,
//! and its VALUE keys the eleven chart values. Each open deal's DEAL key holds its run's first
//! hour, with its terms as an event; each resource's MEMBERS key holds the version of the open
//! deals naming it, the deals themselves an event of each version, and the KNOWN key 1 while
//! the tag's deals are known. A value equal to the last one the key held is not observed again
//! unless it starts a run, as the row accumulator kept them.

use super::{
    RESOURCE_DECIMALS,
    facts::{self, Countries, Deal, Names, Pool},
};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
    hash::FastMap,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A tag's keys: each resource's pool run (by resource), its chart values (by
/// `resource << 4 | value`), whether its deals are known, each resource's open deals' version
/// (by resource) and each deal's run (by the tag's deal ordinal).
pub const POOL: u64 = 0;
pub const VALUE: u64 = 1;
pub const KNOWN: u64 = 2;
pub const MEMBERS: u64 = 3;
pub const DEAL: u64 = 4;
/// A tag's scalars, at `actor * SCALARS + scalar`: the resources it learned, its unnamed and
/// unresolved row-hours, and each resource it ever held pools of with its longest balance bar.
pub const SCALARS: u64 = 4;
pub const NAMES: u64 = 0;
pub const COUNTS: u64 = 1;
pub const SIZES: u64 = 2;
/// Every key's evidence: each walked hour is known.
const DOMAIN: u64 = 0;

/// Event kinds.
pub mod kind {
    /// `[Boolean starts, Integer produced, adjustment, imported, to_export, exported, used,
    /// special_total, production_demand]`, on a POOL key at its run's first hour.
    pub const POOL: u64 = 0;
    /// `[Unsigned deal, Unsigned resource, Boolean export, Unsigned partner actor, Integer
    /// delivered, factories, convoys assigned, convoys required, Boolean starts]`, on a DEAL key
    /// at its run's first hour.
    pub const TRADE: u64 = 1;
    /// `[Unsigned deal key…]`, on a MEMBERS key at each version.
    pub const MEMBERS: u64 = 2;
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved resources emit is not valid")
}

/// A key and the last value it held, as the row accumulator kept it.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
struct Held {
    key: KeyHandle,
    last: Option<i128>,
}

/// A pool run: its first hour, whether it starts a segment, and its pools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Run {
    first: i32,
    starts: bool,
    values: [i64; 8],
}

/// One resource's pools of a tag.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Resource {
    resource: u32,
    /// POOL, then each VALUE, once published.
    keys: Option<[Held; 12]>,
    run: Option<Run>,
    /// The run goes on through the hour before (`pool_keys`).
    open: bool,
    /// The hour a row last went into its run.
    applied: Option<i32>,
    /// It ever held a run, and the longest balance bar its runs drew.
    series: bool,
    size: i128,
}

/// An open deal of a tag: its terms and the run they hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Trade {
    resource: u32,
    export: bool,
    partner: u64,
    delivered: i64,
    factories: i32,
    convoys_assigned: i32,
    convoys_required: i32,
    first: i32,
    starts: bool,
}

impl Trade {
    fn terms(&self) -> (u32, bool, u64, i64, i32, i32, i32) {
        (
            self.resource,
            self.export,
            self.partner,
            self.delivered,
            self.factories,
            self.convoys_assigned,
            self.convoys_required,
        )
    }
}

/// One resource's open deals of a tag, as last published.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Membership {
    resource: u32,
    held: Held,
    members: Vec<KeyHandle>,
    published: bool,
    events: u64,
}

/// What a tag holds now, and its keys.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Country {
    live: bool,
    identity: Option<(u32, u32)>,
    /// The identity it was last observed with.
    track: Option<(u32, u32)>,
    pools_open: bool,
    deals_open: bool,
    /// The first hour of its current known-deals span.
    span: i32,
    learned: BTreeMap<u32, Arc<str>>,
    /// The names it last learned from, by [`Emit::names_epoch`].
    learned_from: Option<u64>,
    resources: Vec<Resource>,
    trades: BTreeMap<u64, Trade>,
    /// Each deal's key by handle and resource, ascending.
    deal_keys: Vec<((u64, u32), Held)>,
    memberships: Vec<Membership>,
    known: Option<Held>,
    unnamed: u64,
    unresolved: u64,
    /// The unnamed pool and deal rows and the unresolved deal rows of each hour, and the walked
    /// hours already counted.
    rates: [u64; 3],
    counted: u64,
}

impl Country {
    fn resource(&mut self, resource: u32) -> &mut Resource {
        let at = match self
            .resources
            .binary_search_by_key(&resource, |held| held.resource)
        {
            Ok(at) => at,
            Err(at) => {
                self.resources.insert(
                    at,
                    Resource {
                        resource,
                        keys: None,
                        run: None,
                        open: false,
                        applied: None,
                        series: false,
                        size: 0,
                    },
                );
                at
            }
        };
        &mut self.resources[at]
    }
}

/// The eleven chart values of a run's pools, as the widget draws them: supply, demand, then
/// produced, from subjects, imported, special projects, production, to the overlord, the
/// export reserve and the rounding left, and the net.
pub fn chart_values(values: &[i64; 8]) -> [i128; 11] {
    let [produced, subjects, imported, reserve, _, used, after_projects, demand] =
        values.map(i128::from);
    let projects = used - after_projects;
    let production = -demand;
    let supply = produced + subjects.max(0) + imported;
    let needs = projects + production + (-subjects).max(0) + reserve;
    let net = after_projects - production;
    [
        supply,
        needs,
        produced,
        subjects.max(0),
        imported,
        -projects,
        -production,
        -(-subjects).max(0),
        -reserve,
        net - (supply - needs),
        net,
    ]
}

/// Whether a holder's pool rows name one resource more than once (rows of two raw countries
/// the links join into its slot).
fn repeats(rows: &[Pool]) -> bool {
    rows.iter()
        .enumerate()
        .any(|(at, pool)| rows[..at].iter().any(|held| held.resource == pool.resource))
}

/// The longest balance bar `native` draws: its parts' magnitudes, needs as one part.
fn size(native: &[i128; 11]) -> i128 {
    let supply = native[2].abs() + native[3].abs() + native[4].abs();
    let demand = (native[5] + native[6]).abs() + native[7].abs() + native[8].abs();
    supply.max(demand)
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    countries: Vec<Country>,
    /// The live tags by holder slot, and each canonical slot's partner tag.
    slots: FastMap<u32, Vec<u64>>,
    partners: FastMap<u32, Option<Arc<str>>>,
    /// The countries last resolved: none after a restart until its first hour.
    resolved: Option<Arc<Countries>>,
    names: Option<Names>,
    names_epoch: u64,
    pools: FastMap<u32, Vec<Pool>>,
    deals: FastMap<u32, Vec<Deal>>,
    complete: [bool; 2],
    /// Hours walked.
    walked: u64,
    /// Actors whose scalars changed since the last cut, and actors counting rows every hour.
    stale: BTreeSet<u64>,
    counting: BTreeSet<u64>,
    last: Option<i32>,
    /// The first hour walked since the last cut or hour not walked.
    first: Option<i32>,
}

impl Default for Emit {
    fn default() -> Self {
        Self::new()
    }
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            countries: Vec::new(),
            slots: FastMap::default(),
            partners: FastMap::default(),
            resolved: None,
            names: None,
            names_epoch: 0,
            pools: FastMap::default(),
            deals: FastMap::default(),
            complete: [false; 2],
            walked: 0,
            stale: BTreeSet::new(),
            counting: BTreeSet::new(),
            last: None,
            first: None,
        }
    }

    fn key(
        &mut self,
        writer: &mut Writer<'_>,
        actor: u64,
        measure: u64,
        dimension: Option<u64>,
        decimals: u8,
    ) -> io::Result<KeyHandle> {
        self.actors.key(
            writer,
            Place {
                actor,
                measure,
                dimension,
            },
            (RunMode::Exact, decimals),
            Some(DOMAIN),
        )
    }

    /// The tag actor named `tag`.
    fn actor(&mut self, writer: &mut Writer<'_>, tag: &str, hour: i32) -> io::Result<u64> {
        if let Some(actor) = self.actors.find(tag.as_bytes()) {
            return Ok(actor);
        }
        let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
        if actor >= (family::ACTORS / SCALARS) {
            return Err(dashboard_core::invalid("Too many resource countries"));
        }
        let index = actor as usize;
        if index >= self.countries.len() {
            self.countries.resize(index + 1, Country::default());
        }
        Ok(actor)
    }

    /// `value` of `key` from `hour` on, as the row accumulator observed it: an unchanged value
    /// not starting a run is not observed again, even after the key ended; a value after an
    /// hour without one starts a run.
    fn observe(
        carry: &mut Carry,
        writer: &mut Writer<'_>,
        held: &mut Held,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()> {
        if starts || value.is_none() || held.last != value {
            let open = carry.held(held.key).is_some();
            carry.value(writer, held.key, hour, value, starts || !open)?;
            if value.is_some() {
                held.last = value;
            }
        }
        Ok(())
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("resources.emit", None);
        let at = hour.hour;
        let gap = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => false,
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "A resources hour does not follow the hour before",
                ));
            }
            Some(last) => {
                self.close(writer, last)?;
                true
            }
            None => true,
        };
        self.last = Some(at);
        self.first.get_or_insert(at);
        self.walked += 1;
        // Each actor observed this hour, with whether its pools and its deals are.
        let mut touched: BTreeMap<u64, [bool; 2]> = BTreeMap::new();
        let mut every = [gap; 2];
        if let Some(facts) = hour.facts {
            let facts::Facts {
                restart,
                countries,
                names,
                pools,
                deals,
                ..
            } = *facts;
            // A segment's first hour restates what it reads: what it equals goes on unobserved.
            if let Some(countries) = countries
                && !(restart && self.resolved.as_deref() == Some(&*countries))
            {
                for actor in self.identify(writer, at, &countries)? {
                    touched.insert(actor, [true; 2]);
                }
                self.resolved = Some(countries);
                every = [true; 2];
            }
            if let Some(names) = names
                && !(restart && names == self.names)
            {
                self.names = names;
                self.names_epoch += 1;
                every = [true; 2];
            }
            if let Some(pools) = pools {
                let slots = Self::list(&mut self.pools, &mut self.complete[0], pools, restart);
                // A restated holder whose rows repeat a resource is observed again, as the row
                // census observed it at every whole read.
                let slots = slots.map(|mut slots| {
                    if restart {
                        slots.extend(
                            self.pools
                                .iter()
                                .filter(|(_, rows)| repeats(rows))
                                .map(|(&slot, _)| slot),
                        );
                        slots.sort_unstable();
                        slots.dedup();
                    }
                    slots
                });
                match slots {
                    None => every[0] = true,
                    Some(slots) => {
                        for slot in slots {
                            for &actor in self.slots.get(&slot).into_iter().flatten() {
                                touched.entry(actor).or_default()[0] = true;
                            }
                        }
                    }
                }
            }
            if let Some(deals) = deals {
                let slots = Self::list(&mut self.deals, &mut self.complete[1], deals, restart);
                match slots {
                    None => every[1] = true,
                    Some(slots) => {
                        for slot in slots {
                            for &actor in self.slots.get(&slot).into_iter().flatten() {
                                touched.entry(actor).or_default()[1] = true;
                            }
                        }
                    }
                }
            }
        }
        if every != [false; 2] {
            for (actor, country) in self.countries.iter().enumerate() {
                if country.live || country.track.is_some() {
                    let flags = touched.entry(actor as u64).or_default();
                    flags[0] |= every[0];
                    flags[1] |= every[1];
                }
            }
        }
        for (actor, flags) in touched {
            self.observe_country(writer, actor, at, flags, gap)?;
        }
        Ok(())
    }

    /// Takes a listed table's rows by holder slot: the slots whose rows changed, or none when
    /// every slot's are observed again (a whole listing but a segment's restatement, or a
    /// change of completeness).
    fn list<T: PartialEq>(
        held: &mut FastMap<u32, Vec<T>>,
        complete: &mut bool,
        listed: facts::Listed<T>,
        restart: bool,
    ) -> Option<Vec<u32>> {
        let became = *complete != listed.complete;
        *complete = listed.complete;
        if listed.whole && restart && !became {
            let mut slots: Vec<u32> = Vec::new();
            let mut kept = FastMap::default();
            for (slot, rows) in listed.slots {
                if held.remove(&slot).as_ref() != Some(&rows) {
                    slots.push(slot);
                }
                if !rows.is_empty() {
                    kept.insert(slot, rows);
                }
            }
            slots.extend(held.keys().copied());
            slots.sort_unstable();
            *held = kept;
            return Some(slots);
        }
        if listed.whole || became {
            held.clear();
            for (slot, rows) in listed.slots {
                if !rows.is_empty() {
                    held.insert(slot, rows);
                }
            }
            return None;
        }
        let mut slots = Vec::with_capacity(listed.slots.len());
        for (slot, rows) in listed.slots {
            if rows.is_empty() {
                held.remove(&slot);
            } else {
                held.insert(slot, rows);
            }
            slots.push(slot);
        }
        Some(slots)
    }

    /// The countries the hour resolves: the actors live before or now.
    fn identify(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        countries: &Countries,
    ) -> io::Result<Vec<u64>> {
        let mut observed = Vec::new();
        for (actor, country) in self.countries.iter_mut().enumerate() {
            if country.live {
                country.live = false;
                country.identity = None;
                observed.push(actor as u64);
            }
        }
        self.slots.clear();
        for (tag, holder) in &countries.tags {
            let actor = self.actor(writer, tag, at)?;
            let country = &mut self.countries[actor as usize];
            country.live = true;
            country.identity = *holder;
            if let Some((slot, _)) = holder {
                self.slots.entry(*slot).or_default().push(actor);
            }
            observed.push(actor);
        }
        self.partners = countries.partners.clone();
        Ok(observed)
    }

    /// Adds the rows a country counted each hour since it was last observed.
    fn settle(&mut self, actor: u64) {
        let walked = self.walked;
        let country = &mut self.countries[actor as usize];
        let hours = walked.saturating_sub(country.counted);
        if hours != 0 && country.rates != [0; 3] {
            country.unnamed += hours * (country.rates[0] + country.rates[1]);
            country.unresolved += hours * country.rates[2];
            self.stale.insert(actor);
        }
        country.counted = walked;
    }

    /// Observes `actor` at `at`: its pools and its deals as `flags` say, as the row census
    /// observes a tag, then publishes what changed.
    fn observe_country(
        &mut self,
        writer: &mut Writer<'_>,
        actor: u64,
        at: i32,
        [pools, deals]: [bool; 2],
        gap: bool,
    ) -> io::Result<()> {
        // The hours before this one count at the rates they had.
        self.walked -= 1;
        self.settle(actor);
        self.walked += 1;
        let names = self.names.clone();
        let names_epoch = self.names_epoch;
        let index = actor as usize;
        let mut stale = false;
        let country = &mut self.countries[index];
        let identity = country.identity.filter(|_| country.live);
        let continues = !gap && identity.is_some() && country.track == identity;
        country.track = identity;
        let pools_continue = continues && country.pools_open;
        let span_continues = !gap && country.deals_open;
        if !(continues && country.deals_open) {
            country.trades.clear();
        }
        if pools {
            country.pools_open = false;
        }
        if deals {
            country.deals_open = false;
        }
        // The resources whose runs went on through the hour before.
        let was_open: Vec<u32> = if pools {
            country
                .resources
                .iter_mut()
                .filter_map(|held| std::mem::take(&mut held.open).then_some(held.resource))
                .collect()
        } else {
            Vec::new()
        };
        let Some(((slot, _), names)) = identity.zip(names.as_ref()) else {
            // As the row census forgets its pools, published only when they were observed.
            for held in &mut country.resources {
                held.open = false;
            }
            country.pools_open = false;
            country.deals_open = false;
            country.trades.clear();
            country.rates = [0; 3];
            self.counting.remove(&actor);
            return self.publish(writer, actor, at, [pools, deals], &was_open);
        };
        if country.learned_from != Some(names_epoch) {
            for (id, token) in names.iter() {
                if !country.learned.contains_key(id) {
                    country.learned.insert(*id, Arc::clone(token));
                    stale = true;
                }
            }
            country.learned_from = Some(names_epoch);
        }
        if pools {
            country.rates[0] = 0;
            if self.complete[0] {
                country.pools_open = true;
                let rows = self.pools.get(&slot).map_or(&[][..], Vec::as_slice);
                for row in rows {
                    if !country.learned.contains_key(&row.resource) {
                        country.rates[0] += 1;
                        continue;
                    }
                    let open = was_open.binary_search(&row.resource).is_ok();
                    let held = country.resource(row.resource);
                    let follows = pools_continue && open && held.applied != Some(at);
                    held.applied = Some(at);
                    if !(follows && held.run.is_some_and(|run| run.values == row.values)) {
                        held.run = Some(Run {
                            first: at,
                            starts: !follows,
                            values: row.values,
                        });
                    }
                    held.open = true;
                    if !held.series {
                        held.series = true;
                        stale = true;
                    }
                }
            }
        }
        if deals {
            country.rates[1] = 0;
            country.rates[2] = 0;
            if self.complete[1] {
                country.deals_open = true;
                if !span_continues {
                    country.span = at;
                }
                let rows = self.deals.get(&slot).map_or(&[][..], Vec::as_slice);
                let mut still = BTreeMap::new();
                for deal in rows {
                    let (export, partner) = if deal.exporter == Some(slot) {
                        (true, deal.importer)
                    } else if deal.importer == Some(slot) {
                        (false, deal.exporter)
                    } else {
                        continue;
                    };
                    if !country.learned.contains_key(&deal.resource) {
                        country.rates[1] += 1;
                        continue;
                    }
                    let Some(partner) = partner
                        .and_then(|partner| self.partners.get(&partner))
                        .and_then(Option::as_deref)
                        .and_then(|tag| self.actors.find(tag.as_bytes()))
                    else {
                        country.rates[2] += 1;
                        continue;
                    };
                    let mut trade = Trade {
                        resource: deal.resource,
                        export,
                        partner,
                        delivered: deal.delivered,
                        factories: deal.factories,
                        convoys_assigned: deal.convoys_assigned,
                        convoys_required: deal.convoys_required,
                        first: at,
                        starts: false,
                    };
                    match country.trades.get(&deal.id) {
                        Some(open) if open.terms() == trade.terms() => trade = *open,
                        previous => trade.starts = previous.is_none(),
                    }
                    still.insert(deal.id, trade);
                }
                country.trades = still;
            } else {
                country.trades.clear();
            }
        }
        let [pool_rows, deal_rows, partners] = country.rates;
        if pool_rows + deal_rows + partners != 0 {
            country.unnamed += pool_rows + deal_rows;
            country.unresolved += partners;
            stale = true;
            self.counting.insert(actor);
        } else {
            self.counting.remove(&actor);
        }
        country.counted = self.walked;
        if stale {
            self.stale.insert(actor);
        }
        self.publish(writer, actor, at, [pools, deals], &was_open)
    }

    /// Publishes `actor`'s pools and its deals at `at`, as `flags` say: every resource whose run
    /// went on through the hour before (`was_open`) or goes on now.
    fn publish(
        &mut self,
        writer: &mut Writer<'_>,
        actor: u64,
        at: i32,
        [pools, deals]: [bool; 2],
        was_open: &[u32],
    ) -> io::Result<()> {
        if pools {
            let resources: Vec<u32> = self.countries[actor as usize]
                .resources
                .iter()
                .filter(|held| held.open || was_open.binary_search(&held.resource).is_ok())
                .map(|held| held.resource)
                .collect();
            for resource in resources {
                self.publish_pool(writer, actor, resource, at)?;
            }
        }
        if deals {
            self.publish_deals(writer, actor, at)?;
        }
        Ok(())
    }

    fn publish_pool(
        &mut self,
        writer: &mut Writer<'_>,
        actor: u64,
        resource: u32,
        at: i32,
    ) -> io::Result<()> {
        let index = actor as usize;
        if self.countries[index].resource(resource).keys.is_none() {
            let mut keys = [Held::default(); 12];
            keys[0].key = self.key(writer, actor, POOL, Some(u64::from(resource)), 0)?;
            for (value, held) in keys.iter_mut().enumerate().skip(1) {
                let dimension = u64::from(resource) << 4 | (value as u64 - 1);
                held.key = self.key(writer, actor, VALUE, Some(dimension), RESOURCE_DECIMALS)?;
            }
            self.countries[index].resource(resource).keys = Some(keys);
        }
        let named = self
            .names
            .as_ref()
            .is_some_and(|names| names.binary_search_by_key(&resource, |&(id, _)| id).is_ok());
        let held = self.countries[index].resource(resource);
        let run = held.run.filter(|_| held.open);
        let mut keys = held.keys.expect("published pool keys");
        let starts = run.is_some_and(|run| run.first == at && run.starts);
        if let Some(run) = run
            && keys[0].last != Some(i128::from(run.first))
        {
            let mut cells = Vec::with_capacity(9);
            cells.push(Cell::Boolean(run.starts));
            cells.extend(run.values.iter().map(|&value| Cell::Integer(i128::from(value))));
            let fields = writer.cells(cells)?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key: keys[0].key,
                native_hour: at,
                read_hour: None,
                sequence,
                kind: kind::POOL,
                fields,
            })?;
        }
        let carry = &mut self.carry;
        Self::observe(
            carry,
            writer,
            &mut keys[0],
            at,
            run.map(|run| i128::from(run.first)),
            starts,
        )?;
        let native = run.map(|run| chart_values(&run.values));
        for (value, held) in keys.iter_mut().skip(1).enumerate() {
            let amount = native.filter(|_| named).map(|native| native[value]);
            Self::observe(carry, writer, held, at, amount, starts)?;
        }
        let held = self.countries[index].resource(resource);
        held.keys = Some(keys);
        if let Some(native) = native {
            let size = size(&native);
            if size > held.size {
                held.size = size;
                self.stale.insert(actor);
            }
        }
        Ok(())
    }

    fn publish_deals(&mut self, writer: &mut Writer<'_>, actor: u64, at: i32) -> io::Result<()> {
        let index = actor as usize;
        let (open, span) = {
            let country = &self.countries[index];
            (country.deals_open, country.span)
        };
        let starts = open && span == at;
        if self.countries[index].known.is_none() {
            let key = self.key(writer, actor, KNOWN, None, 0)?;
            self.countries[index].known = Some(Held { key, last: None });
        }
        let mut known = self.countries[index].known.expect("known key");
        Self::observe(
            &mut self.carry,
            writer,
            &mut known,
            at,
            open.then_some(1),
            starts,
        )?;
        self.countries[index].known = Some(known);
        // Each open deal's run, and the deal keys of each resource.
        let mut active: BTreeMap<u32, Vec<KeyHandle>> = BTreeMap::new();
        let mut changed = BTreeSet::new();
        let trades: Vec<(u64, Trade)> = self.countries[index]
            .trades
            .iter()
            .map(|(&deal, &trade)| (deal, trade))
            .collect();
        for (deal, trade) in trades {
            let place = (deal, trade.resource);
            let found = self.countries[index]
                .deal_keys
                .binary_search_by_key(&place, |&(place, _)| place);
            let mut held = match found {
                Ok(at_key) => self.countries[index].deal_keys[at_key].1,
                Err(at_key) => {
                    let ordinal = self.countries[index].deal_keys.len() as u64;
                    let key = self.key(writer, actor, DEAL, Some(ordinal), 0)?;
                    let held = Held { key, last: None };
                    self.countries[index].deal_keys.insert(at_key, (place, held));
                    held
                }
            };
            if held.last != Some(i128::from(trade.first)) {
                let fields = writer.cells([
                    Cell::Unsigned(deal),
                    Cell::Unsigned(u64::from(trade.resource)),
                    Cell::Boolean(trade.export),
                    Cell::Unsigned(trade.partner),
                    Cell::Integer(i128::from(trade.delivered)),
                    Cell::Integer(i128::from(trade.factories)),
                    Cell::Integer(i128::from(trade.convoys_assigned)),
                    Cell::Integer(i128::from(trade.convoys_required)),
                    Cell::Boolean(trade.starts),
                ])?;
                let sequence = writer.sequence()?;
                writer.event(Event {
                    key: held.key,
                    native_hour: at,
                    read_hour: None,
                    sequence,
                    kind: kind::TRADE,
                    fields,
                })?;
                changed.insert(trade.resource);
            }
            Self::observe(
                &mut self.carry,
                writer,
                &mut held,
                at,
                Some(i128::from(trade.first)),
                trade.first == at && trade.starts,
            )?;
            if let Ok(at_key) = self.countries[index]
                .deal_keys
                .binary_search_by_key(&place, |&(place, _)| place)
            {
                self.countries[index].deal_keys[at_key].1 = held;
            }
            active.entry(trade.resource).or_default().push(held.key);
        }
        // Each resource it learned: its open deals' version.
        let learned: Vec<u32> = self.countries[index].learned.keys().copied().collect();
        for resource in learned {
            let at_member = match self.countries[index]
                .memberships
                .binary_search_by_key(&resource, |held| held.resource)
            {
                Ok(at_member) => at_member,
                Err(at_member) => {
                    let key = self.key(writer, actor, MEMBERS, Some(u64::from(resource)), 0)?;
                    self.countries[index].memberships.insert(
                        at_member,
                        Membership {
                            resource,
                            held: Held { key, last: None },
                            members: Vec::new(),
                            published: false,
                            events: 0,
                        },
                    );
                    at_member
                }
            };
            let mut members = active.remove(&resource).unwrap_or_default();
            members.sort_unstable();
            let membership = &mut self.countries[index].memberships[at_member];
            for &removed in &membership.members {
                if !members.contains(&removed) {
                    self.carry.value(writer, removed, at, None, false)?;
                }
            }
            if !membership.published || membership.members != members || changed.contains(&resource)
            {
                let fields = writer.cells(members.iter().map(|&key| Cell::Unsigned(key)))?;
                let sequence = writer.sequence()?;
                writer.event(Event {
                    key: membership.held.key,
                    native_hour: at,
                    read_hour: None,
                    sequence,
                    kind: kind::MEMBERS,
                    fields,
                })?;
                membership.events += 1;
                membership.members = members;
                membership.published = true;
            }
            let version = i128::from(membership.events);
            Self::observe(
                &mut self.carry,
                writer,
                &mut membership.held,
                at,
                open.then_some(version),
                starts,
            )?;
        }
        Ok(())
    }

    /// Ends every run after `last`, the hour before one not walked, and the hours known.
    fn close(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        self.known(writer, last)
    }

    /// The hours walked since the last cut or hour not walked, through `last`, as known.
    fn known(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        if let Some(first) = self.first.take() {
            writer.domain(
                DOMAIN,
                Evidence {
                    span: Span::new(first, last)?,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the scalars that changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        self.known(writer, last)?;
        let counting: Vec<u64> = self.counting.iter().copied().collect();
        for actor in counting {
            self.settle(actor);
        }
        for actor in std::mem::take(&mut self.stale) {
            let country = &self.countries[actor as usize];
            let base = actor * SCALARS;
            let names = Scalar::List(
                country
                    .learned
                    .iter()
                    .map(|(&id, token)| {
                        Scalar::List(vec![
                            Scalar::Unsigned(u64::from(id)),
                            Scalar::Text(token.as_bytes().into()),
                        ])
                    })
                    .collect(),
            );
            let counts = Scalar::List(vec![
                Scalar::Unsigned(country.unnamed),
                Scalar::Unsigned(country.unresolved),
            ]);
            let sizes = Scalar::List(
                country
                    .resources
                    .iter()
                    .filter(|held| held.series)
                    .map(|held| {
                        Scalar::List(vec![
                            Scalar::Unsigned(u64::from(held.resource)),
                            Scalar::Integer(held.size),
                        ])
                    })
                    .collect(),
            );
            for (id, scalar) in [(NAMES, names), (COUNTS, counts), (SIZES, sizes)] {
                let cell = family::cell(writer, &scalar)?;
                writer.catalogue(base + id, cell)?;
            }
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection names its countries, names and rows anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            countries: self
                .countries
                .iter()
                .map(|country| Country {
                    // Its rates count from the restart's first hour.
                    counted: 0,
                    ..country.clone()
                })
                .collect(),
            names: self.names.as_ref().map(|names| names.to_vec()),
            names_epoch: self.names_epoch,
            complete: self.complete,
            last: self.last,
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        if saved.countries.len() > directory.actors.len() {
            return Err(invalid());
        }
        emit.countries = saved.countries;
        emit.countries.resize(directory.actors.len(), Country::default());
        for (actor, country) in emit.countries.iter().enumerate() {
            if country.live
                && let Some((slot, _)) = country.identity
            {
                emit.slots.entry(slot).or_default().push(actor as u64);
            }
            if country.rates != [0; 3] {
                emit.counting.insert(actor as u64);
            }
        }
        emit.names = saved.names.map(Into::into);
        emit.names_epoch = saved.names_epoch;
        emit.complete = saved.complete;
        emit.last = saved.last;
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    countries: Vec<Country>,
    names: Option<Vec<(u32, Arc<str>)>>,
    names_epoch: u64,
    complete: [bool; 2],
    last: Option<i32>,
}
