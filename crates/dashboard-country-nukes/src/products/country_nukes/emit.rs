//! Country nukes' emit: each tag's stockpile as runs on the family's series publication (its
//! total, its composition version with the composition as an event, and each type's count),
//! each rank's holder by total, the complete reads' schedule and whether each read's stockpiles
//! were empty, each refused read's hours by reason, and each stockpile increase as an event.
//!
//! A tag holds its holder slot's latest read while it keeps its holder, an hour between reads
//! carrying it: its runs go on while it keeps its holder and every hour holds a stockpile. A
//! holder change, an unknown or incomplete read and an hour not walked start a segment; a tag
//! leaving the countries ends its runs. Ranks order the nonzero known totals descending, ties by
//! tag.

use super::facts::{self, Census, Countries, Stock, Stocks};
use super::NukeCount;
use dashboard_series::{
    Blob, Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
    hash::FastMap,
};
use replay_data::{cadence, format::FormatVersion, rows::TableId};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashSet},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A tag's total; its composition version, each version's composition a [`kind::MIX`] event;
/// and each type's count, by ordinal.
pub const TOTAL: u64 = 0;
pub const MIX: u64 = 1;
pub const PARTS: u64 = 2;
/// The family's keys: the tag at each rank by total (by position), the complete reads'
/// schedule (its period, from each read off schedule), whether each read's stockpiles were
/// empty, each refused read's hours (by reason) and the stockpile increases.
pub const POSITIONS: u64 = 0;
pub const READS: u64 = 1;
pub const EMPTY: u64 = 2;
pub const REFUSALS: u64 = 3;
pub const INCREASES: u64 = 4;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// Family scalars: the ordinals read, each native type's spelling, each refused ordinal's rows,
/// and the unresolved rows and tags and the country-hours with an unknown total; and each
/// refusal's reason, by id.
pub const TYPES: u64 = 0;
pub const TOKENS: u64 = 1;
pub const REFUSED: u64 = 2;
pub const COUNTS: u64 = 3;
pub const REASONS: u64 = 1 << 20;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// Event kinds.
pub mod kind {
    /// `[List [Unsigned ordinal, Integer value]…, Integer total | Null]`, on a tag's MIX key.
    pub const MIX: u64 = 0;
    /// `[Unsigned actor, Unsigned slot, Unsigned id, Unsigned amount]`, on the family's
    /// INCREASES key.
    pub const INCREASE: u64 = 1;
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved nukes emit is not valid")
}

/// The complete reads' schedule: a read is due a period after the one before.
fn period(format: FormatVersion) -> i32 {
    match cadence::cadence_in(TableId::Nukes, format) {
        cadence::Cadence::Daily(_) => cadence::STAGGER_PERIOD,
        cadence::Cadence::Hourly | cadence::Cadence::Sliced => 1,
        cadence::Cadence::Static => 0,
    }
}

/// What a tag holds now, and its keys.
#[derive(Clone, Default)]
struct Country {
    /// TOTAL and MIX.
    keys: [KeyHandle; 2],
    /// Each type's PARTS key, by ordinal.
    parts: Vec<(u32, KeyHandle)>,
    /// Its holder while it is live.
    holder: Option<(u32, u32)>,
    stock: Option<Arc<Stock>>,
    version: u64,
}

/// The ranking by total: the ranked tags in rank order with their totals, each one's total by
/// tag, and each position's key.
#[derive(Default)]
struct Ranking {
    order: Vec<(u64, i32)>,
    amounts: FastMap<u64, i32>,
    positions: Vec<KeyHandle>,
}

/// An increase the hour read: its tag, holder and amount.
type Increase = (u64, (u32, u32), u32);

/// The family's emit on one branch.
pub struct Emit {
    period: i32,
    actors: Actors,
    carry: Carry,
    /// Each actor's name, and what each tag holds, by actor.
    labels: Vec<Arc<[u8]>>,
    countries: Vec<Country>,
    /// The live tags with their holders, in tag order, and by holder slot.
    live: Vec<(u64, (u32, u32))>,
    slots: FastMap<u32, Vec<(u64, (u32, u32))>>,
    ranking: Ranking,
    /// READS, EMPTY and INCREASES.
    family: Option<[KeyHandle; 3]>,
    /// Each refusal's id and key, by reason, the key refusing the hour before, and why the
    /// table is not complete.
    reasons: BTreeMap<String, (u64, KeyHandle)>,
    refusing: Option<KeyHandle>,
    reason: Option<String>,
    /// The next read due on the schedule.
    next: Option<i32>,
    /// Whether the countries and the Nukes table are complete.
    resolved: bool,
    complete: bool,
    /// The countries' unresolved tags, and the last read's counts.
    unreadable_tags: usize,
    census: Census,
    /// The live tags holding a stockpile whose total is unknown.
    unknown: u64,
    types: BTreeSet<u32>,
    tokens: BTreeMap<u32, String>,
    refused: BTreeMap<u32, u64>,
    unreadable: u64,
    unknown_hours: u64,
    /// The scalars changed since the last cut.
    stale: bool,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
}

/// A stockpile's composition as event fields.
fn mix_cells(writer: &mut Writer<'_>, stock: &Stock) -> io::Result<Blob> {
    let mut cells = Vec::with_capacity(stock.counts.len() + 1);
    for count in &stock.counts {
        let pair = writer.cells([
            Cell::Unsigned(u64::from(count.ordinal)),
            Cell::Integer(i128::from(count.value)),
        ])?;
        cells.push(Cell::List(pair));
    }
    cells.push(
        stock
            .total
            .map_or(Cell::Null, |total| Cell::Integer(i128::from(total))),
    );
    writer.cells(cells)
}

/// The ranked total of a stockpile: a nonzero known one.
fn ranked(stock: Option<&Stock>) -> Option<i32> {
    stock.and_then(|stock| stock.total).filter(|&total| total != 0)
}

impl Emit {
    pub fn new(format: FormatVersion) -> Self {
        Self {
            period: period(format),
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            labels: Vec::new(),
            countries: Vec::new(),
            live: Vec::new(),
            slots: FastMap::default(),
            ranking: Ranking::default(),
            family: None,
            reasons: BTreeMap::new(),
            refusing: None,
            reason: None,
            next: None,
            resolved: false,
            complete: false,
            unreadable_tags: 0,
            census: Census::default(),
            unknown: 0,
            types: BTreeSet::new(),
            tokens: BTreeMap::new(),
            refused: BTreeMap::new(),
            unreadable: 0,
            unknown_hours: 0,
            stale: true,
            last: None,
            first: None,
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))
    }

    fn family(&mut self, writer: &mut Writer<'_>) -> io::Result<[KeyHandle; 3]> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let mut keys = [0; 3];
        for (key, measure) in keys.iter_mut().zip([READS, EMPTY, INCREASES]) {
            *key = self.key(
                writer,
                Place {
                    actor: FAMILY,
                    measure,
                    dimension: None,
                },
            )?;
        }
        self.family = Some(keys);
        Ok(keys)
    }

    /// The tag actor named `tag`, with its keys.
    fn country(&mut self, writer: &mut Writer<'_>, tag: &str, hour: i32) -> io::Result<u64> {
        if let Some(actor) = self.actors.find(tag.as_bytes()) {
            return Ok(actor);
        }
        let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
        let index = actor as usize;
        if index >= self.countries.len() {
            self.countries.resize(index + 1, Country::default());
            self.labels.resize(index + 1, Arc::from(&[][..]));
        }
        self.labels[index] = Arc::from(tag.as_bytes());
        let mut keys = [0; 2];
        for (key, measure) in keys.iter_mut().zip([TOTAL, MIX]) {
            *key = self.key(
                writer,
                Place {
                    actor,
                    measure,
                    dimension: None,
                },
            )?;
        }
        self.countries[index].keys = keys;
        Ok(actor)
    }

    /// `actor`'s PARTS key of `ordinal`.
    fn part(&mut self, writer: &mut Writer<'_>, actor: u64, ordinal: u32) -> io::Result<KeyHandle> {
        let parts = &self.countries[actor as usize].parts;
        let at = parts.partition_point(|&(held, _)| held < ordinal);
        if let Some(&(held, key)) = parts.get(at)
            && held == ordinal
        {
            return Ok(key);
        }
        let key = self.key(
            writer,
            Place {
                actor,
                measure: PARTS,
                dimension: Some(u64::from(ordinal)),
            },
        )?;
        self.countries[actor as usize].parts.insert(at, (ordinal, key));
        Ok(key)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("nukes.emit", None);
        let at = hour.hour;
        let adjacent = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => true,
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "A nukes hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every run.
            Some(last) => {
                self.close(writer, last)?;
                false
            }
            None => false,
        };
        self.last = Some(at);
        self.first.get_or_insert(at);
        let [reads, empty, _] = self.family(writer)?;
        let mut touched = Vec::new();
        let mut increases = Vec::new();
        let mut read = hour.read && self.complete;
        if let Some(facts) = hour.facts {
            let facts::Facts {
                complete,
                reason,
                countries,
                stocks,
                census,
                tokens,
                ..
            } = *facts;
            for (ordinal, spelling) in tokens {
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    self.tokens.entry(ordinal)
                {
                    entry.insert(spelling);
                    self.stale = true;
                }
            }
            if let Some(census) = census {
                // The ordinals a read holds are read with its counts.
                for &ordinal in &census.types {
                    self.stale |= self.types.insert(ordinal);
                }
                self.census = census;
            }
            if let Some(reason) = reason {
                self.reason = Some(reason);
            }
            // Every live tag is read anew where the countries or the table's completeness
            // changed, else the tags at the slots the read names.
            let anew = countries.is_some() || complete != self.complete;
            self.complete = complete;
            read = hour.read && complete;
            if let Some(countries) = countries {
                self.identify(writer, at, countries.as_deref(), &mut touched)?;
            }
            let stocks = stocks.filter(|_| read && self.resolved);
            if anew || stocks.as_ref().is_some_and(|stocks| stocks.whole) {
                for at_live in 0..self.live.len() {
                    let (actor, holder) = self.live[at_live];
                    let stock = self.stock_of(actor, holder, adjacent, read, stocks.as_ref());
                    let offer = (actor, holder, stock);
                    self.offer(writer, at, adjacent, offer, &mut touched, &mut increases)?;
                }
            } else if let Some(stocks) = stocks {
                for (slot, stock) in &stocks.slots {
                    let Some(holders) = self.slots.get(slot) else {
                        continue;
                    };
                    for at_slot in 0..holders.len() {
                        let (actor, holder) = self.slots[slot][at_slot];
                        let offer = (actor, holder, stock.clone());
                        self.offer(writer, at, adjacent, offer, &mut touched, &mut increases)?;
                    }
                }
            }
        }
        self.rank(writer, at, &touched)?;
        // In tag order, as the row census observes them.
        let labels = &self.labels;
        increases.sort_unstable_by(|left: &Increase, right: &Increase| {
            labels[left.0 as usize].cmp(&labels[right.0 as usize])
        });
        for increase in increases {
            self.increase(writer, at, increase)?;
        }
        self.count(read);
        self.schedule(writer, reads, at, read)?;
        if read {
            let flag = self.resolved && self.census.empty;
            self.carry
                .value(writer, empty, at, Some(i128::from(flag)), false)?;
        }
        let refused = hour.read && !self.complete;
        self.refuse(writer, at, refused)
    }

    /// The countries the hour resolves, none while they are not complete: tags leaving end
    /// their runs.
    fn identify(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        countries: Option<&Countries>,
        touched: &mut Vec<u64>,
    ) -> io::Result<()> {
        self.resolved = countries.is_some();
        self.unreadable_tags = countries.map_or(0, |countries| countries.unreadable);
        let mut live = Vec::with_capacity(countries.map_or(0, |countries| countries.tags.len()));
        for (tag, holder) in countries.into_iter().flat_map(|countries| &countries.tags) {
            live.push((self.country(writer, tag, at)?, *holder));
        }
        let kept: HashSet<u64> = live.iter().map(|&(actor, _)| actor).collect();
        for (actor, _) in std::mem::take(&mut self.live) {
            if !kept.contains(&actor) {
                self.end(writer, at, actor)?;
                self.countries[actor as usize].holder = None;
                touched.push(actor);
            }
        }
        self.slots.clear();
        for &(actor, holder) in &live {
            self.slots.entry(holder.0).or_default().push((actor, holder));
        }
        self.live = live;
        Ok(())
    }

    /// What `actor`, now of `holder`, holds this hour: none while the table is not complete;
    /// its slot's stockpile at a read naming it; else what it held the hour before, while it
    /// keeps its holder.
    fn stock_of(
        &self,
        actor: u64,
        holder: (u32, u32),
        adjacent: bool,
        read: bool,
        stocks: Option<&Stocks>,
    ) -> Option<Arc<Stock>> {
        if !self.complete {
            return None;
        }
        if read && let Some(stocks) = stocks {
            match stocks
                .slots
                .binary_search_by_key(&holder.0, |&(slot, _)| slot)
            {
                Ok(at) => return stocks.slots[at].1.clone(),
                Err(_) if stocks.whole => return None,
                Err(_) => {}
            }
        }
        let country = &self.countries[actor as usize];
        country
            .stock
            .clone()
            .filter(|_| adjacent && country.holder == Some(holder))
    }

    /// Ends what `actor`'s runs hold.
    fn end(&mut self, writer: &mut Writer<'_>, at: i32, actor: u64) -> io::Result<()> {
        let country = &mut self.countries[actor as usize];
        let Some(stock) = country.stock.take() else {
            return Ok(());
        };
        self.unknown -= u64::from(stock.total.is_none());
        let keys = country.keys;
        for key in keys {
            self.carry.value(writer, key, at, None, false)?;
        }
        for count in &stock.counts {
            let key = self.part(writer, actor, count.ordinal)?;
            self.carry.value(writer, key, at, None, false)?;
        }
        Ok(())
    }

    /// `actor`'s stockpile from `at` on, of `holder`: an equal one going on changes nothing;
    /// another starts a run of its composition, a segment where it does not go on from the
    /// hour before, and an increase where its total grows within one.
    fn offer(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        adjacent: bool,
        (actor, holder, stock): (u64, (u32, u32), Option<Arc<Stock>>),
        touched: &mut Vec<u64>,
        increases: &mut Vec<Increase>,
    ) -> io::Result<()> {
        let country = &self.countries[actor as usize];
        let continues =
            adjacent && country.holder == Some(holder) && country.stock.is_some();
        let Some(stock) = stock else {
            self.end(writer, at, actor)?;
            self.countries[actor as usize].holder = Some(holder);
            touched.push(actor);
            return Ok(());
        };
        if continues
            && country
                .stock
                .as_ref()
                .is_some_and(|held| Arc::ptr_eq(held, &stock) || **held == *stock)
        {
            self.countries[actor as usize].holder = Some(holder);
            return Ok(());
        }
        let before = country.stock.clone().filter(|_| continues);
        let starts = !continues;
        if let (Some(was), Some(now)) = (before.as_ref().and_then(|held| held.total), stock.total)
            && let Ok(amount) = u32::try_from(i64::from(now) - i64::from(was))
            && amount > 0
        {
            increases.push((actor, holder, amount));
        }
        let held = self.countries[actor as usize].stock.take();
        self.unknown -= u64::from(held.as_ref().is_some_and(|held| held.total.is_none()));
        self.unknown += u64::from(stock.total.is_none());
        let country = &mut self.countries[actor as usize];
        country.version += 1;
        let version = country.version;
        let [total, mix] = country.keys;
        self.carry
            .value(writer, mix, at, Some(i128::from(version)), starts)?;
        let fields = mix_cells(writer, &stock)?;
        let sequence = writer.sequence()?;
        writer.event(Event {
            key: mix,
            native_hour: at,
            read_hour: None,
            sequence,
            kind: kind::MIX,
            fields,
        })?;
        let held_total = before.as_ref().and_then(|held| held.total);
        self.carry.value(
            writer,
            total,
            at,
            stock.total.map(i128::from),
            starts || held_total.is_none(),
        )?;
        // Each type it held or holds now, its count none where it holds no row of it.
        let held_counts = before
            .as_ref()
            .map_or(&[][..], |held| held.counts.as_slice());
        let mut ordinals: Vec<u32> = held_counts
            .iter()
            .chain(&stock.counts)
            .map(|count| count.ordinal)
            .collect();
        if let Some(held) = &held {
            ordinals.extend(held.counts.iter().map(|count| count.ordinal));
        }
        ordinals.sort_unstable();
        ordinals.dedup();
        for ordinal in ordinals {
            let key = self.part(writer, actor, ordinal)?;
            let find = |counts: &[NukeCount]| {
                counts
                    .binary_search_by_key(&ordinal, |count| count.ordinal)
                    .ok()
                    .map(|at| counts[at].value)
            };
            let was = find(held_counts);
            let now = find(stock.counts.as_slice());
            self.carry
                .value(writer, key, at, now.map(i128::from), starts || was.is_none())?;
        }
        let country = &mut self.countries[actor as usize];
        country.holder = Some(holder);
        country.stock = Some(stock);
        touched.push(actor);
        Ok(())
    }

    /// The increase's event.
    fn increase(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        (actor, (slot, id), amount): Increase,
    ) -> io::Result<()> {
        let [_, _, key] = self.family(writer)?;
        let fields = writer.cells([
            Cell::Unsigned(actor),
            Cell::Unsigned(u64::from(slot)),
            Cell::Unsigned(u64::from(id)),
            Cell::Unsigned(u64::from(amount)),
        ])?;
        let sequence = writer.sequence()?;
        writer.event(Event {
            key,
            native_hour: at,
            read_hour: Some(at),
            sequence,
            kind: kind::INCREASE,
            fields,
        })
    }

    /// The order of `left` before `right`: larger first, ties by tag.
    fn before(labels: &[Arc<[u8]>], left: (u64, i32), right: (u64, i32)) -> Ordering {
        right
            .1
            .cmp(&left.1)
            .then_with(|| labels[left.0 as usize].cmp(&labels[right.0 as usize]))
    }

    /// Moves the `touched` tags in the ranking to their totals' places; the positions that
    /// changed go on from `hour`.
    fn rank(&mut self, writer: &mut Writer<'_>, hour: i32, touched: &[u64]) -> io::Result<()> {
        let (mut leaving, mut entering) = (Vec::new(), Vec::new());
        for &actor in touched {
            let amount = ranked(self.countries[actor as usize].stock.as_deref());
            let held = self.ranking.amounts.get(&actor).copied();
            if held == amount {
                continue;
            }
            leaving.extend(held.map(|held| (actor, held)));
            entering.extend(amount.map(|amount| (actor, amount)));
            match amount {
                Some(amount) => self.ranking.amounts.insert(actor, amount),
                None => self.ranking.amounts.remove(&actor),
            };
        }
        if leaving.is_empty() && entering.is_empty() {
            return Ok(());
        }
        let labels = &self.labels;
        let order = |left: &(u64, i32), right: &(u64, i32)| Self::before(labels, *left, *right);
        leaving.sort_unstable_by(order);
        entering.sort_unstable_by(order);
        let ranking = &mut self.ranking;
        let misplaced = || io::Error::other("The nuclear ranking lost its order");
        // Every place before the first leaving tag's and the first entering tag's keeps its tag.
        let mut low = ranking.order.len();
        if let Some(first) = leaving.first() {
            low = ranking
                .order
                .binary_search_by(|held| order(held, first))
                .map_err(|_| misplaced())?;
        }
        if let Some(first) = entering.first() {
            let Err(at) = ranking.order.binary_search_by(|held| order(held, first)) else {
                return Err(misplaced());
            };
            low = low.min(at);
        }
        let old = ranking.order.split_off(low);
        let (mut at, mut leaving, mut entering) = (0, &leaving[..], &entering[..]);
        while !leaving.is_empty() || !entering.is_empty() {
            let held = old.get(at);
            if held.is_some() && held == leaving.first() {
                (at, leaving) = (at + 1, &leaving[1..]);
            } else if let Some(&entry) = entering.first()
                && held.is_none_or(|held| order(held, &entry) == Ordering::Greater)
            {
                ranking.order.push(entry);
                entering = &entering[1..];
            } else {
                ranking.order.push(*held.ok_or_else(misplaced)?);
                at += 1;
            }
        }
        let merged = ranking.order.len();
        ranking.order.extend_from_slice(&old[at..]);
        // Past the merge, a tag keeps its place unless the ranking grew or shrank.
        let end = if ranking.order.len() == low + old.len() {
            merged
        } else {
            ranking.order.len().max(low + old.len())
        };
        for position in low..end {
            let was = old.get(position - low).map(|&(actor, _)| actor);
            let now = self.ranking.order.get(position).map(|&(actor, _)| actor);
            if was == now {
                continue;
            }
            let key = match self.ranking.positions.get(position) {
                Some(&key) => key,
                None => {
                    let key = self.key(
                        writer,
                        Place {
                            actor: FAMILY,
                            measure: POSITIONS,
                            dimension: Some(position as u64),
                        },
                    )?;
                    self.ranking.positions.push(key);
                    key
                }
            };
            self.carry
                .value(writer, key, hour, now.map(i128::from), false)?;
        }
        Ok(())
    }

    /// The hour's counts, as the row census counts every hour it observes.
    fn count(&mut self, read: bool) {
        let mut unreadable = 0;
        if self.resolved {
            unreadable += self.unreadable_tags as u64;
            if read {
                unreadable += self.census.unreadable as u64;
                for &(ordinal, count) in &self.census.refused {
                    *self.refused.entry(ordinal).or_default() += count as u64;
                    self.stale = true;
                }
            }
        }
        if unreadable != 0 || self.unknown != 0 {
            self.unreadable += unreadable;
            self.unknown_hours += self.unknown;
            self.stale = true;
        }
    }

    /// The complete reads' schedule at `at`: a read off schedule starts a run of its period, a
    /// read due and missed ends it.
    fn schedule(
        &mut self,
        writer: &mut Writer<'_>,
        key: KeyHandle,
        at: i32,
        read: bool,
    ) -> io::Result<()> {
        if read {
            if self.next != Some(at) {
                self.carry
                    .value(writer, key, at, Some(i128::from(self.period)), true)?;
            }
            self.next = (self.period > 0).then(|| at + self.period);
        } else if self.next == Some(at) {
            self.carry.value(writer, key, at, None, false)?;
            self.next = None;
        }
        Ok(())
    }

    /// A read of a table not complete (`refused`) under its reason; an hour refusing nothing
    /// ends the one before's.
    fn refuse(&mut self, writer: &mut Writer<'_>, at: i32, refused: bool) -> io::Result<()> {
        let key = match self.reason.as_ref().filter(|_| refused) {
            Some(reason) => Some(match self.reasons.get(reason) {
                Some(&(_, key)) => key,
                None => {
                    let reason = reason.clone();
                    let id = self.reasons.len() as u64;
                    let key = self.key(
                        writer,
                        Place {
                            actor: FAMILY,
                            measure: REFUSALS,
                            dimension: Some(id),
                        },
                    )?;
                    let text = family::cell(writer, &Scalar::Text(reason.as_bytes().into()))?;
                    writer.catalogue(REASONS + id, text)?;
                    self.reasons.insert(reason, (id, key));
                    key
                }
            }),
            None => None,
        };
        if self.refusing != key {
            if let Some(held) = self.refusing {
                self.carry.value(writer, held, at, None, false)?;
            }
            if let Some(key) = key {
                self.carry.value(writer, key, at, Some(1), false)?;
            }
            self.refusing = key;
        }
        Ok(())
    }

    /// Ends every run after `last`, the hour before one not walked.
    fn close(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        for country in &mut self.countries {
            country.stock = None;
        }
        self.ranking.order.clear();
        self.ranking.amounts.clear();
        self.unknown = 0;
        self.next = None;
        self.refusing = None;
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the scalars that changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
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
        if self.stale {
            let pairs = |pairs: &mut dyn Iterator<Item = (u64, Scalar)>| {
                Scalar::List(
                    pairs
                        .map(|(key, value)| Scalar::List(vec![Scalar::Unsigned(key), value]))
                        .collect(),
                )
            };
            let scalars = [
                (
                    TYPES,
                    Scalar::List(
                        self.types
                            .iter()
                            .map(|&ordinal| Scalar::Unsigned(u64::from(ordinal)))
                            .collect(),
                    ),
                ),
                (
                    TOKENS,
                    pairs(&mut self.tokens.iter().map(|(&ordinal, spelling)| {
                        (
                            u64::from(ordinal),
                            Scalar::Text(spelling.as_bytes().into()),
                        )
                    })),
                ),
                (
                    REFUSED,
                    pairs(
                        &mut self
                            .refused
                            .iter()
                            .map(|(&ordinal, &count)| (u64::from(ordinal), Scalar::Unsigned(count))),
                    ),
                ),
                (
                    COUNTS,
                    Scalar::List(vec![
                        Scalar::Unsigned(self.unreadable),
                        Scalar::Unsigned(self.unknown_hours),
                    ]),
                ),
            ];
            for (id, scalar) in scalars {
                let cell = family::cell(writer, &scalar)?;
                writer.catalogue(id, cell)?;
            }
            self.stale = false;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection names its countries anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            last: self.last,
            next: self.next,
            refusing: self.refusing.map(|key| {
                self.reasons
                    .values()
                    .find(|&&(_, held)| held == key)
                    .map_or(u64::MAX, |&(id, _)| id)
            }),
            resolved: self.resolved,
            complete: self.complete,
            reason: self.reason.clone(),
            unreadable_tags: self.unreadable_tags,
            census: self.census.clone(),
            live: self.live.clone(),
            countries: self
                .countries
                .iter()
                .enumerate()
                .filter(|(_, country)| country.holder.is_some() || country.version != 0)
                .map(|(actor, country)| SavedCountry {
                    actor: actor as u64,
                    holder: country.holder,
                    stock: country.stock.as_deref().map(|stock| {
                        (
                            stock
                                .counts
                                .iter()
                                .map(|count| (count.ordinal, count.value))
                                .collect(),
                            stock.total,
                        )
                    }),
                    version: country.version,
                })
                .collect(),
            types: self.types.iter().copied().collect(),
            tokens: self.tokens.clone(),
            refused: self.refused.clone(),
            unreadable: self.unreadable,
            unknown_hours: self.unknown_hours,
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(format: FormatVersion, directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new(format);
        emit.stale = false;
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.labels = directory
            .actors
            .iter()
            .map(|actor| Arc::from(&actor.name[..]))
            .collect();
        emit.countries = vec![Country::default(); directory.actors.len()];
        for (actor, country) in emit.countries.iter_mut().enumerate() {
            let actor = actor as u64;
            for (key, measure) in country.keys.iter_mut().zip([TOTAL, MIX]) {
                *key = directory.key(actor, measure, None).ok_or_else(invalid)?;
            }
            country.parts = directory
                .dimensions(actor, PARTS)
                .iter()
                .map(|&(ordinal, key)| {
                    u32::try_from(ordinal)
                        .map(|ordinal| (ordinal, key))
                        .map_err(|_| invalid())
                })
                .collect::<io::Result<_>>()?;
        }
        let mut family = [0; 3];
        for (key, measure) in family.iter_mut().zip([READS, EMPTY, INCREASES]) {
            *key = directory.key(FAMILY, measure, None).ok_or_else(invalid)?;
        }
        emit.family = Some(family);
        emit.ranking.positions = directory
            .dimensions(FAMILY, POSITIONS)
            .iter()
            .enumerate()
            .map(|(at, &(position, key))| (position == at as u64).then_some(key).ok_or_else(invalid))
            .collect::<io::Result<_>>()?;
        let mut id = 0;
        while let Some(scalar) = directory.scalar(REASONS + id) {
            let reason = std::str::from_utf8(scalar.text().ok_or_else(invalid)?)
                .map_err(|_| invalid())?
                .to_owned();
            let key = directory
                .key(FAMILY, REFUSALS, Some(id))
                .ok_or_else(invalid)?;
            emit.reasons.insert(reason, (id, key));
            id += 1;
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.next = saved.next;
        emit.refusing = match saved.refusing {
            Some(id) => Some(
                emit.reasons
                    .values()
                    .find(|&&(held, _)| held == id)
                    .map(|&(_, key)| key)
                    .ok_or_else(invalid)?,
            ),
            None => None,
        };
        emit.resolved = saved.resolved;
        emit.complete = saved.complete;
        emit.reason = saved.reason;
        emit.unreadable_tags = saved.unreadable_tags;
        emit.census = saved.census;
        emit.types = saved.types.into_iter().collect();
        emit.tokens = saved.tokens;
        emit.refused = saved.refused;
        emit.unreadable = saved.unreadable;
        emit.unknown_hours = saved.unknown_hours;
        for country in saved.countries {
            let held = emit
                .countries
                .get_mut(country.actor as usize)
                .ok_or_else(invalid)?;
            held.holder = country.holder;
            held.stock = country.stock.map(|(counts, total)| {
                Arc::new(Stock {
                    counts: counts
                        .into_iter()
                        .map(|(ordinal, value)| NukeCount { ordinal, value })
                        .collect(),
                    total,
                })
            });
            held.version = country.version;
            emit.unknown += u64::from(
                held.stock
                    .as_ref()
                    .is_some_and(|stock| stock.total.is_none()),
            );
        }
        for &(actor, holder) in &saved.live {
            if actor as usize >= emit.countries.len() {
                return Err(invalid());
            }
            emit.slots.entry(holder.0).or_default().push((actor, holder));
        }
        emit.live = saved.live;
        // The ranking holds every tag with a ranked total, in rank order.
        let mut known: Vec<(u64, i32)> = emit
            .countries
            .iter()
            .enumerate()
            .filter_map(|(actor, country)| Some((actor as u64, ranked(country.stock.as_deref())?)))
            .collect();
        let labels = &emit.labels;
        known.sort_unstable_by(|&left, &right| Self::before(labels, left, right));
        emit.ranking.amounts = known.iter().copied().collect();
        emit.ranking.order = known;
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct SavedCountry {
    actor: u64,
    holder: Option<(u32, u32)>,
    stock: Option<(Vec<(u32, i32)>, Option<i32>)>,
    version: u64,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    next: Option<i32>,
    refusing: Option<u64>,
    resolved: bool,
    complete: bool,
    reason: Option<String>,
    unreadable_tags: usize,
    census: Census,
    live: Vec<(u64, (u32, u32))>,
    countries: Vec<SavedCountry>,
    types: Vec<u32>,
    tokens: BTreeMap<u32, String>,
    refused: BTreeMap<u32, u64>,
    unreadable: u64,
    unknown_hours: u64,
}
