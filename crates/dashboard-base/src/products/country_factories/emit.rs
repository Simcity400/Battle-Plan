//! Country factories' emit: each tag's values as runs on the family's series publication (its
//! composition version with the composition as an event, its supplying read, each ranked count
//! and each part), each rank's holder by each ranked count, each hour's refused reads, and the
//! counts the reply warns of.
//!
//! A tag holds its holder slot's latest read while it keeps its holder, an hour between reads
//! carrying it: its runs go on while it keeps its holder and every hour holds values. A holder
//! change, an unknown or incomplete read and an hour not walked start a segment; a tag leaving
//! the countries ends its runs. Ranks order every valued tag by count descending, ties by tag.

use super::{
    CHANNELS, FIELD_COUNT,
    facts::{self, Census, Countries, Stocks, Values},
};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
    hash::FastMap,
};
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, collections::HashSet, io, sync::Arc};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A tag's composition version, each version's values a [`kind::MIX`] event; its supplying
/// read ([`HOURLY`], or the hour of the read it holds); each ranked count by channel
/// ([`super::channel`]); and each part by [`super::fields`] ordinal.
pub const MIX: u64 = 0;
pub const READS: u64 = 1;
pub const VALUES: u64 = 2;
pub const PARTS: u64 = 3;
/// The family's keys: each hour's refused reads (unresolved, repeated or unshowable rows and
/// tags that are not text), and the tag at each rank of channel `c` (`RANKS + c`, by position).
pub const REFUSED: u64 = 0;
pub const RANKS: u64 = 16;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// Family scalars: the country-hours whose `unavailable` is not the sum of its parts by group,
/// and the unreadable rows and unnamed country-hours.
pub const UNEXPLAINED: u64 = 0;
pub const COUNTS: u64 = 1;
/// A READS value: the values were read this hour.
pub const HOURLY: i128 = i32::MAX as i128 + 1;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;
/// A tag's keys: MIX, READS, the ranked counts and the parts.
const KEYS: usize = 2 + CHANNELS + FIELD_COUNT;

/// Event kinds.
pub mod kind {
    /// `[Integer value…]` in [`super::fields`] order, on a tag's MIX key.
    pub const MIX: u64 = 0;
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved factories emit is not valid")
}

/// Where a tag's values were read: this hour, or held from the read at an hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Supply {
    Hourly,
    Held(i32),
}

impl Supply {
    fn value(self) -> i128 {
        match self {
            Self::Hourly => HOURLY,
            Self::Held(hour) => i128::from(hour),
        }
    }
}

/// What a tag holds now, and its keys.
#[derive(Clone, Default)]
struct Country {
    /// Made with its first values.
    keys: Option<Box<[KeyHandle; KEYS]>>,
    /// Its holder while it is live.
    holder: Option<(u32, u32)>,
    values: Option<Values>,
    counts: Option<[i64; CHANNELS]>,
    supply: Option<Supply>,
    version: u64,
}

/// One channel's ranking: the valued tags in rank order with their counts, and each position's
/// key.
#[derive(Default)]
struct Ranking {
    order: Vec<(u64, i64)>,
    positions: Vec<KeyHandle>,
}

/// A tag whose counts may have changed this hour, with the counts it held before.
type Touched = (u64, Option<[i64; CHANNELS]>);

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// Each actor's name, and what each tag holds, by actor.
    labels: Vec<Arc<[u8]>>,
    countries: Vec<Country>,
    /// The live tags with their holders, in tag order, and by holder slot.
    live: Vec<(u64, (u32, u32))>,
    slots: FastMap<u32, Vec<(u64, (u32, u32))>>,
    rankings: Vec<Ranking>,
    refused: Option<KeyHandle>,
    /// Whether the countries and the CountryFactories table are complete.
    resolved: bool,
    complete: bool,
    /// The hour before's read flag.
    read: Option<bool>,
    /// The last counts read.
    census: Census,
    unexplained: [u64; 3],
    unreadable: u64,
    unnamed: u64,
    /// The scalars changed since the last cut.
    stale: bool,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
}

/// The order of `left` before `right`: larger first, ties by tag.
fn before(labels: &[Arc<[u8]>], left: &(u64, i64), right: &(u64, i64)) -> Ordering {
    right
        .1
        .cmp(&left.1)
        .then_with(|| labels[left.0 as usize].cmp(&labels[right.0 as usize]))
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            labels: Vec::new(),
            countries: Vec::new(),
            live: Vec::new(),
            slots: FastMap::default(),
            rankings: (0..CHANNELS).map(|_| Ranking::default()).collect(),
            refused: None,
            resolved: false,
            complete: false,
            read: None,
            census: Census::default(),
            unexplained: [0; 3],
            unreadable: 0,
            unnamed: 0,
            stale: true,
            last: None,
            first: None,
        }
    }

    fn key(&mut self, writer: &mut Writer, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))
    }

    fn refused(&mut self, writer: &mut Writer) -> io::Result<KeyHandle> {
        if let Some(key) = self.refused {
            return Ok(key);
        }
        let key = self.key(
            writer,
            Place {
                actor: FAMILY,
                measure: REFUSED,
                dimension: None,
            },
        )?;
        self.refused = Some(key);
        Ok(key)
    }

    /// The tag actor named `tag`.
    fn country(&mut self, writer: &mut Writer, tag: &str, hour: i32) -> io::Result<u64> {
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
        Ok(actor)
    }

    /// `actor`'s keys, made with its first values.
    fn keys(&mut self, writer: &mut Writer, actor: u64) -> io::Result<[KeyHandle; KEYS]> {
        if let Some(keys) = &self.countries[actor as usize].keys {
            return Ok(**keys);
        }
        let mut keys = [0; KEYS];
        let places = [(MIX, None), (READS, None)]
            .into_iter()
            .chain((0..CHANNELS as u64).map(|channel| (VALUES, Some(channel))))
            .chain((0..FIELD_COUNT as u64).map(|ordinal| (PARTS, Some(ordinal))));
        for (key, (measure, dimension)) in keys.iter_mut().zip(places) {
            *key = self.key(
                writer,
                Place {
                    actor,
                    measure,
                    dimension,
                },
            )?;
        }
        self.countries[actor as usize].keys = Some(Box::new(keys));
        Ok(keys)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("factories.emit", None);
        let at = hour.hour;
        let adjacent = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => true,
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "A factories hour does not follow the hour before",
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
        let refused = self.refused(writer)?;
        // A read flag unlike the hour before's dates every live tag's values anew.
        let mut anew = adjacent && self.read != Some(hour.read);
        self.read = Some(hour.read);
        let mut touched = Vec::new();
        let mut stocks = None;
        if let Some(facts) = hour.facts {
            let facts::Facts {
                complete,
                countries,
                stocks: read,
                census,
                ..
            } = *facts;
            if let Some(census) = census {
                self.census = census;
            }
            anew |= countries.is_some() || complete != self.complete;
            self.complete = complete;
            if let Some(countries) = countries {
                self.identify(writer, at, countries.as_deref(), &mut touched)?;
            }
            stocks = read.filter(|_| hour.read && self.resolved);
        }
        if anew || stocks.as_ref().is_some_and(|stocks| stocks.whole) {
            for at_live in 0..self.live.len() {
                let (actor, holder) = self.live[at_live];
                let values = self.values_of(actor, holder, adjacent, hour.read, stocks.as_ref());
                let offer = (actor, holder, values);
                self.offer(writer, at, (adjacent, hour.read), offer, &mut touched)?;
            }
        } else if let Some(stocks) = &stocks {
            for (slot, values) in &stocks.slots {
                let Some(holders) = self.slots.get(slot) else {
                    continue;
                };
                for at_slot in 0..holders.len() {
                    let (actor, holder) = self.slots[slot][at_slot];
                    let offer = (actor, holder, values.clone());
                    self.offer(writer, at, (adjacent, hour.read), offer, &mut touched)?;
                }
            }
        }
        self.rank(writer, at, &touched)?;
        self.count(writer, refused, at, adjacent, hour.read)
    }

    /// The countries the hour resolves, none while they are not complete: tags leaving end
    /// their runs.
    fn identify(
        &mut self,
        writer: &mut Writer,
        at: i32,
        countries: Option<&Countries>,
        touched: &mut Vec<Touched>,
    ) -> io::Result<()> {
        self.resolved = countries.is_some();
        let mut live = Vec::with_capacity(countries.map_or(0, |countries| countries.tags.len()));
        for (tag, holder) in countries.into_iter().flat_map(|countries| &countries.tags) {
            live.push((self.country(writer, tag, at)?, *holder));
        }
        let kept: HashSet<u64> = live.iter().map(|&(actor, _)| actor).collect();
        for (actor, _) in std::mem::take(&mut self.live) {
            if !kept.contains(&actor) {
                self.end(writer, at, actor, touched)?;
                self.countries[actor as usize].holder = None;
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
    /// its slot's values at a read naming it; else what it held the hour before, while it
    /// keeps its holder.
    fn values_of(
        &self,
        actor: u64,
        holder: (u32, u32),
        adjacent: bool,
        read: bool,
        stocks: Option<&Stocks>,
    ) -> Option<Values> {
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
            .values
            .clone()
            .filter(|_| adjacent && country.holder == Some(holder))
    }

    /// Ends what `actor`'s runs hold.
    fn end(
        &mut self,
        writer: &mut Writer,
        at: i32,
        actor: u64,
        touched: &mut Vec<Touched>,
    ) -> io::Result<()> {
        let country = &mut self.countries[actor as usize];
        if country.values.take().is_none() {
            return Ok(());
        }
        touched.push((actor, country.counts.take()));
        country.supply = None;
        let keys = country.keys.as_deref().copied().ok_or_else(invalid)?;
        for key in keys {
            self.carry.value(writer, key, at, None, false)?;
        }
        Ok(())
    }

    /// `actor`'s values from `at` on, of `holder`, read this hour or not: equal values going
    /// on from the same read change nothing; others start a run of their composition, a
    /// segment where they do not go on from the hour before.
    fn offer(
        &mut self,
        writer: &mut Writer,
        at: i32,
        (adjacent, read): (bool, bool),
        (actor, holder, values): (u64, (u32, u32), Option<Values>),
        touched: &mut Vec<Touched>,
    ) -> io::Result<()> {
        let country = &self.countries[actor as usize];
        let continues = adjacent && country.holder == Some(holder) && country.values.is_some();
        let Some(values) = values else {
            self.end(writer, at, actor, touched)?;
            self.countries[actor as usize].holder = Some(holder);
            return Ok(());
        };
        // An hour between reads holds the last read: the hour before's, or the one it held.
        let supply = if read {
            Supply::Hourly
        } else {
            match country.supply.filter(|_| continues) {
                Some(Supply::Hourly) => Supply::Held(at - 1),
                Some(held) => held,
                None => Supply::Held(at),
            }
        };
        let same = continues
            && country
                .values
                .as_ref()
                .is_some_and(|held| Arc::ptr_eq(held, &values) || **held == *values);
        if same && country.supply == Some(supply) {
            self.countries[actor as usize].holder = Some(holder);
            return Ok(());
        }
        let keys = self.keys(writer, actor)?;
        let starts = !continues;
        if !same {
            let country = &mut self.countries[actor as usize];
            country.version += 1;
            let version = country.version;
            self.carry
                .value(writer, keys[0], at, Some(i128::from(version)), starts)?;
            let cells = writer.cells(values.iter().map(|&value| Cell::Integer(i128::from(value))))?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key: keys[0],
                native_hour: at,
                read_hour: None,
                sequence,
                kind: kind::MIX,
                fields: cells,
            })?;
            let counts = super::channel_counts(&values);
            for (&key, &count) in keys[2..2 + CHANNELS].iter().zip(&counts) {
                self.carry
                    .value(writer, key, at, Some(i128::from(count)), starts)?;
            }
            for (&key, &value) in keys[2 + CHANNELS..].iter().zip(values.iter()) {
                self.carry
                    .value(writer, key, at, Some(i128::from(value)), starts)?;
            }
            let country = &mut self.countries[actor as usize];
            if country.counts != Some(counts) {
                touched.push((actor, country.counts));
                country.counts = Some(counts);
            }
            country.values = Some(values);
        }
        self.carry
            .value(writer, keys[1], at, Some(supply.value()), starts)?;
        let country = &mut self.countries[actor as usize];
        country.supply = Some(supply);
        country.holder = Some(holder);
        Ok(())
    }

    /// Moves the `touched` tags in each ranking to their counts' places; the positions that
    /// changed go on from `hour`.
    fn rank(&mut self, writer: &mut Writer, hour: i32, touched: &[Touched]) -> io::Result<()> {
        if touched.is_empty() {
            return Ok(());
        }
        let misplaced = || io::Error::other("A factory ranking lost its order");
        for channel in 0..CHANNELS {
            let labels = &self.labels;
            let ranking = &mut self.rankings[channel];
            let held = ranking.order.len();
            let (mut low, mut high, mut moves) = (usize::MAX, 0, 0);
            for &(actor, was) in touched {
                let was = was.map(|counts| counts[channel]);
                let now = self.countries[actor as usize]
                    .counts
                    .map(|counts| counts[channel]);
                if was == now {
                    continue;
                }
                if let Some(was) = was {
                    let at = ranking
                        .order
                        .binary_search_by(|entry| before(labels, entry, &(actor, was)))
                        .map_err(|_| misplaced())?;
                    ranking.order.remove(at);
                    (low, high, moves) = (low.min(at), high.max(at), moves + 1);
                }
                if let Some(now) = now {
                    let Err(at) = ranking
                        .order
                        .binary_search_by(|entry| before(labels, entry, &(actor, now)))
                    else {
                        return Err(misplaced());
                    };
                    ranking.order.insert(at, (actor, now));
                    (low, high, moves) = (low.min(at), high.max(at), moves + 1);
                }
            }
            if moves == 0 {
                continue;
            }
            // Past every move, a tag keeps its place unless the ranking grew or shrank.
            let end = if ranking.order.len() == held {
                (high + moves + 1).min(held)
            } else {
                ranking.order.len().max(held)
            };
            for position in low..end {
                let now = self.rankings[channel]
                    .order
                    .get(position)
                    .map(|&(actor, _)| i128::from(actor));
                let key = match self.rankings[channel].positions.get(position) {
                    Some(&key) => key,
                    None => {
                        let key = self.key(
                            writer,
                            Place {
                                actor: FAMILY,
                                measure: RANKS + channel as u64,
                                dimension: Some(position as u64),
                            },
                        )?;
                        self.rankings[channel].positions.push(key);
                        key
                    }
                };
                if self.carry.held(key) != now {
                    self.carry.value(writer, key, hour, now, false)?;
                }
            }
        }
        Ok(())
    }

    /// The hour's counts, as the row scan counts every hour it observes: tags that are not
    /// text every hour, the rest at a read.
    fn count(
        &mut self,
        writer: &mut Writer,
        key: KeyHandle,
        at: i32,
        adjacent: bool,
        read: bool,
    ) -> io::Result<()> {
        let census = self.census;
        let unreadable = if read { census.unreadable } else { 0 };
        if unreadable != 0 || census.unnamed != 0 {
            self.unreadable += unreadable;
            self.unnamed += census.unnamed;
            self.stale = true;
        }
        if read && census.unexplained != [0; 3] {
            for (total, count) in self.unexplained.iter_mut().zip(census.unexplained) {
                *total += count;
            }
            self.stale = true;
        }
        let refused = i128::from(unreadable + census.unnamed);
        self.carry.value(writer, key, at, Some(refused), !adjacent)?;
        Ok(())
    }

    /// Ends every run after `last`, the hour before one not walked.
    fn close(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        for country in &mut self.countries {
            country.values = None;
            country.counts = None;
            country.supply = None;
        }
        for ranking in &mut self.rankings {
            ranking.order.clear();
        }
        self.read = None;
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the scalars that changed.
    pub fn cut(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
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
            let unsigned = |values: &[u64]| {
                Scalar::List(values.iter().map(|&value| Scalar::Unsigned(value)).collect())
            };
            let scalars = [
                (UNEXPLAINED, unsigned(&self.unexplained)),
                (COUNTS, unsigned(&[self.unreadable, self.unnamed])),
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
            read: self.read,
            resolved: self.resolved,
            complete: self.complete,
            census: self.census,
            live: self.live.clone(),
            countries: self
                .countries
                .iter()
                .enumerate()
                .filter(|(_, country)| country.holder.is_some() || country.version != 0)
                .map(|(actor, country)| SavedCountry {
                    actor: actor as u64,
                    holder: country.holder,
                    values: country.values.as_deref().map(<[i32]>::to_vec),
                    supply: country.supply,
                    version: country.version,
                })
                .collect(),
            unexplained: self.unexplained,
            unreadable: self.unreadable,
            unnamed: self.unnamed,
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
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
            let Some(mix) = directory.key(actor, MIX, None) else {
                continue;
            };
            let mut keys = [0; KEYS];
            keys[0] = mix;
            keys[1] = directory.key(actor, READS, None).ok_or_else(invalid)?;
            for channel in 0..CHANNELS {
                keys[2 + channel] = directory
                    .key(actor, VALUES, Some(channel as u64))
                    .ok_or_else(invalid)?;
            }
            for ordinal in 0..FIELD_COUNT {
                keys[2 + CHANNELS + ordinal] = directory
                    .key(actor, PARTS, Some(ordinal as u64))
                    .ok_or_else(invalid)?;
            }
            country.keys = Some(Box::new(keys));
        }
        emit.refused = Some(directory.key(FAMILY, REFUSED, None).ok_or_else(invalid)?);
        for (channel, ranking) in emit.rankings.iter_mut().enumerate() {
            ranking.positions = directory
                .dimensions(FAMILY, RANKS + channel as u64)
                .iter()
                .enumerate()
                .map(|(at, &(position, key))| {
                    (position == at as u64).then_some(key).ok_or_else(invalid)
                })
                .collect::<io::Result<_>>()?;
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.read = saved.read;
        emit.resolved = saved.resolved;
        emit.complete = saved.complete;
        emit.census = saved.census;
        emit.unexplained = saved.unexplained;
        emit.unreadable = saved.unreadable;
        emit.unnamed = saved.unnamed;
        for country in saved.countries {
            let held = emit
                .countries
                .get_mut(country.actor as usize)
                .ok_or_else(invalid)?;
            if country.values.as_ref().is_some_and(|values| {
                values.len() != FIELD_COUNT || held.keys.is_none()
            }) {
                return Err(invalid());
            }
            held.holder = country.holder;
            held.counts = country.values.as_deref().map(super::channel_counts);
            held.values = country.values.map(Values::from);
            held.supply = country.supply;
            held.version = country.version;
        }
        for &(actor, holder) in &saved.live {
            if actor as usize >= emit.countries.len() {
                return Err(invalid());
            }
            emit.slots.entry(holder.0).or_default().push((actor, holder));
        }
        emit.live = saved.live;
        // Each ranking holds every valued tag, in rank order.
        let labels = &emit.labels;
        for (channel, ranking) in emit.rankings.iter_mut().enumerate() {
            let mut order: Vec<(u64, i64)> = emit
                .countries
                .iter()
                .enumerate()
                .filter_map(|(actor, country)| Some((actor as u64, country.counts?[channel])))
                .collect();
            order.sort_unstable_by(|left, right| before(labels, left, right));
            ranking.order = order;
        }
        Ok(emit)
    }
}

impl Default for Emit {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Serialize, Deserialize)]
struct SavedCountry {
    actor: u64,
    holder: Option<(u32, u32)>,
    values: Option<Vec<i32>>,
    supply: Option<Supply>,
    version: u64,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    read: Option<bool>,
    resolved: bool,
    complete: bool,
    census: Census,
    live: Vec<(u64, (u32, u32))>,
    countries: Vec<SavedCountry>,
    unexplained: [u64; 3],
    unreadable: u64,
    unnamed: u64,
}
