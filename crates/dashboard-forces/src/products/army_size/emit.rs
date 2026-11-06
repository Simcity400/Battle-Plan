//! Army size's emit: each country's eight totals, whether any is known, its composition
//! version with the composition as an event, each total's rank and each rank's holder, and
//! the aid links of volunteers, expeditionaries and host tallies, as runs on the family's
//! series publication.
//!
//! A country's totals go on while it keeps its holder and every hour reads them; a holder
//! change, an unknown hour or an unread hour starts a segment. A tag leaving the live
//! countries ends its runs. Ranks order the known totals descending, ties by tag.

use super::{
    Mix, Totals,
    aid::{self, Party, Sender},
    facts,
};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 2;
/// A country's totals are measures 0..8 ([`super::channel`]); then whether any is known.
pub const COVERAGE: u64 = 8;
/// A country's composition version; each version's composition is a [`MIX_KIND`] event.
pub const MIX: u64 = 9;
/// A country's rank (1 is the largest) by each total: measures 10..18.
pub const RANKS: u64 = 10;
/// The country at each rank, by total: measures 18..26 of [`FAMILY`], by position.
pub const POSITIONS: u64 = 18;
/// Whether each aid channel's sources were read and resolved: [`FAMILY`], by channel.
pub const AID_READ: u64 = 26;
/// Whether an aid party's channel was resolved (dimensions 0..3) and whether it was present
/// (dimension 3).
pub const AID_PROOF: u64 = 27;
/// A sender's divisions at a host: dimension link * 3 + channel ([`LINKS`]).
pub const AID_VALUE: u64 = 28;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
pub const MIX_KIND: u64 = 1;
/// Family scalars: what the hours read, the counts of unresolved reads, refused table reads
/// by reason, and each aid channel's warnings by reason.
pub const AVAILABLE: u64 = 0;
pub const COUNTS: u64 = 1;
pub const INVALID: u64 = 2;
pub const AID_WARNINGS: u64 = 3;
/// Each aid link's sender, host and channels, by link.
pub const LINKS: u64 = 1 << 20;
/// Each sub-unit name a composition counts, by id.
pub const NAMES: u64 = 1 << 26;
/// Every army key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// Strength is fixed5; every other total is a count.
pub fn decimals(channel: usize) -> u8 {
    if matches!(channel, 1 | 3) { 5 } else { 0 }
}

/// An aid party's actor name: a byte no tag starts with, its slot, id and tag.
pub fn party_name(slot: u32, id: u32, tag: &str) -> Vec<u8> {
    let mut name = Vec::with_capacity(9 + tag.len());
    name.push(0xFF);
    name.extend_from_slice(&slot.to_le_bytes());
    name.extend_from_slice(&id.to_le_bytes());
    name.extend_from_slice(tag.as_bytes());
    name
}

/// The slot, id and tag of an aid party's actor name.
pub fn party_of(name: &[u8]) -> Option<(u32, u32, &str)> {
    let rest = name.strip_prefix(&[0xFF])?;
    if rest.len() < 8 {
        return None;
    }
    let slot = u32::from_le_bytes(rest[..4].try_into().ok()?);
    let id = u32::from_le_bytes(rest[4..8].try_into().ok()?);
    Some((slot, id, std::str::from_utf8(&rest[8..]).ok()?))
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved army emit is not valid")
}

/// What a country's last projection held, and its keys.
#[derive(Clone, Default)]
struct Country {
    /// Values 0..8, coverage, composition version, ranks by channel.
    keys: Vec<KeyHandle>,
    /// The holder of its open runs; None once they ended.
    holder: Option<(u32, u32)>,
    totals: Option<Totals>,
    mix: Option<Arc<Mix>>,
    version: u64,
}

/// One total's ranking: its known countries in rank order, and each one's rank.
#[derive(Default)]
struct Ranking {
    order: Vec<u64>,
    amounts: HashMap<u64, i128>,
    positions: Vec<KeyHandle>,
}

/// The family's emit on one branch.
#[derive(Default)]
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// Each projection index's actor, for the segment under way.
    named: Vec<Option<u64>>,
    /// Each actor's name, by id.
    labels: Vec<Arc<[u8]>>,
    /// By actor id; aid parties hold none.
    countries: Vec<Country>,
    live: HashSet<u64>,
    rankings: [Ranking; 8],
    subunits: HashMap<String, u64>,
    parties: HashMap<Party, u64>,
    /// Each aid party's proof keys: channels 0..3, then present.
    proofs: HashMap<u64, [KeyHandle; 4]>,
    reads: Option<[KeyHandle; 3]>,
    present: BTreeSet<u64>,
    current: HashMap<u64, Sender>,
    links: Vec<(u64, u64, u8, [KeyHandle; 3])>,
    link_keys: HashMap<(u64, u64), u64>,
    changed_links: BTreeSet<u64>,
    warnings: [BTreeMap<String, usize>; 3],
    available: [bool; 5],
    unreadable: u64,
    composition: u64,
    invalid: BTreeMap<String, usize>,
    /// The scalars changed since the last cut.
    stale: bool,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            stale: true,
            ..Self::default()
        }
    }

    fn key(
        &mut self,
        writer: &mut Writer,
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

    /// The actor named `name`, first seen at `hour`.
    fn actor(&mut self, writer: &mut Writer, name: &[u8], hour: i32) -> io::Result<u64> {
        if let Some(actor) = self.actors.find(name) {
            return Ok(actor);
        }
        let actor = self.actors.actor(writer, name, hour)?;
        if actor as usize >= self.labels.len() {
            self.labels.resize(actor as usize + 1, Arc::from(&[][..]));
            self.countries
                .resize(actor as usize + 1, Country::default());
        }
        self.labels[actor as usize] = Arc::from(name);
        Ok(actor)
    }

    /// The country actor of `tag`, with its keys.
    fn country(&mut self, writer: &mut Writer, tag: &str, hour: i32) -> io::Result<u64> {
        let actor = self.actor(writer, tag.as_bytes(), hour)?;
        if self.countries[actor as usize].keys.is_empty() {
            let mut keys = Vec::with_capacity(18);
            for channel in 0..8 {
                keys.push(self.key(writer, actor, channel as u64, None, decimals(channel))?);
            }
            keys.push(self.key(writer, actor, COVERAGE, None, 0)?);
            keys.push(self.key(writer, actor, MIX, None, 0)?);
            for channel in 0..8 {
                keys.push(self.key(writer, actor, RANKS + channel, None, 0)?);
            }
            self.countries[actor as usize].keys = keys;
        }
        Ok(actor)
    }

    /// The aid party actor of `party`, with its proof keys.
    fn party(&mut self, writer: &mut Writer, party: &Party, hour: i32) -> io::Result<u64> {
        if let Some(&actor) = self.parties.get(party) {
            return Ok(actor);
        }
        let name = party_name(party.slot, party.id, &party.tag);
        let actor = self.actor(writer, &name, hour)?;
        let mut keys = [0; 4];
        for (channel, key) in keys.iter_mut().enumerate() {
            *key = self.key(writer, actor, AID_PROOF, Some(channel as u64), 0)?;
        }
        self.proofs.insert(actor, keys);
        self.parties.insert(party.clone(), actor);
        Ok(actor)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if self.reads.is_none() {
            let mut reads = [0; 3];
            for (channel, key) in reads.iter_mut().enumerate() {
                *key = self.key(writer, FAMILY, AID_READ, Some(channel as u64), 0)?;
            }
            self.reads = Some(reads);
        }
        if let Some(last) = self.last.filter(|_| !adjacent) {
            // An unread hour ends every run; the next projections start their segments.
            self.carry.close(writer, last)?;
            for country in &mut self.countries {
                country.holder = None;
                country.totals = None;
                country.mix = None;
            }
            for ranking in &mut self.rankings {
                ranking.order.clear();
                ranking.amounts.clear();
            }
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        for (index, tag) in &hour.named {
            let actor = self.country(writer, tag, at)?;
            let index = *index as usize;
            if index >= self.named.len() {
                self.named.resize(index + 1, None);
            }
            self.named[index] = Some(actor);
        }
        let named = |named: &[Option<u64>], index: u32| {
            named
                .get(index as usize)
                .copied()
                .flatten()
                .ok_or_else(|| dashboard_core::invalid("An army country was never named"))
        };
        let mut touched = Vec::new();
        let mut removed = Vec::new();
        if let Some(live) = &hour.live {
            let live: HashSet<u64> = live
                .iter()
                .map(|&index| named(&self.named, index))
                .collect::<io::Result<_>>()?;
            removed.extend(self.live.difference(&live).copied());
            self.live = live;
        }
        for country in &hour.countries {
            let actor = named(&self.named, country.index)?;
            self.offer(writer, at, adjacent, actor, country)?;
            touched.push(actor);
        }
        for actor in removed {
            self.end(writer, at, actor)?;
            touched.push(actor);
        }
        self.rank(writer, at, &touched)?;
        let read = [
            hour.divisions,
            hour.strength,
            hour.manpower,
            hour.battalions,
            hour.fallback,
        ];
        for (held, read) in self.available.iter_mut().zip(read) {
            if read && !*held {
                *held = true;
                self.stale = true;
            }
        }
        if hour.unreadable != 0 || hour.composition_unreadable != 0 || !hour.invalid.is_empty() {
            self.unreadable += hour.unreadable as u64;
            self.composition += hour.composition_unreadable as u64;
            for (reason, count) in hour.invalid {
                *self.invalid.entry(reason).or_default() += count;
            }
            self.stale = true;
        }
        self.aid(writer, at, !adjacent, hour.aid)
    }

    fn end(&mut self, writer: &mut Writer, hour: i32, actor: u64) -> io::Result<()> {
        let country = &mut self.countries[actor as usize];
        for &key in &country.keys[..10] {
            self.carry.value(writer, key, hour, None, false)?;
        }
        country.holder = None;
        country.totals = None;
        country.mix = None;
        Ok(())
    }

    fn offer(
        &mut self,
        writer: &mut Writer,
        hour: i32,
        adjacent: bool,
        actor: u64,
        projected: &facts::Country,
    ) -> io::Result<()> {
        let Some(totals) = projected.totals else {
            return self.end(writer, hour, actor);
        };
        let country = &mut self.countries[actor as usize];
        let continues = adjacent && country.holder == Some(projected.holder);
        let starts = !continues;
        if starts || country.totals != Some(totals) {
            for (channel, value) in totals.into_iter().enumerate() {
                let key = country.keys[channel];
                let starts = starts || self.carry.held(key).is_none();
                self.carry.value(writer, key, hour, value, starts)?;
            }
            self.carry.value(
                writer,
                country.keys[COVERAGE as usize],
                hour,
                Some(1),
                starts,
            )?;
        }
        let key = country.keys[MIX as usize];
        match &projected.mix {
            Some(mix)
                if continues
                    && country
                        .mix
                        .as_ref()
                        .is_some_and(|held| Arc::ptr_eq(held, mix) || **held == **mix) =>
            {
                country.mix = Some(Arc::clone(mix));
            }
            Some(mix) => {
                country.version += 1;
                let version = country.version;
                country.mix = Some(Arc::clone(mix));
                self.carry
                    .value(writer, key, hour, Some(i128::from(version)), starts)?;
                let fields = self.mix_cells(writer, mix)?;
                let sequence = writer.sequence()?;
                writer.event(Event {
                    key,
                    native_hour: hour,
                    read_hour: None,
                    sequence,
                    kind: MIX_KIND,
                    fields,
                })?;
            }
            None => {
                self.carry.value(writer, key, hour, None, false)?;
                country.mix = None;
            }
        }
        let country = &mut self.countries[actor as usize];
        country.holder = Some(projected.holder);
        country.totals = Some(totals);
        Ok(())
    }

    /// A composition as event fields: each attribution's (name id, battalions) pairs, or null
    /// where it is unknown.
    fn mix_cells(
        &mut self,
        writer: &mut Writer,
        mix: &Mix,
    ) -> io::Result<dashboard_series::Blob> {
        let mut sides = Vec::with_capacity(2);
        for side in mix {
            let Some(parts) = side else {
                sides.push(Cell::Null);
                continue;
            };
            let mut cells = Vec::with_capacity(parts.len() * 2);
            for (name, &count) in parts {
                let id = match self.subunits.get(name) {
                    Some(&id) => id,
                    None => {
                        let id = self.subunits.len() as u64;
                        let text = family::cell(writer, &Scalar::Text(name.as_bytes().into()))?;
                        writer.catalogue(NAMES + id, text)?;
                        self.subunits.insert(name.clone(), id);
                        id
                    }
                };
                cells.push(Cell::Unsigned(id));
                cells.push(Cell::Integer(i128::from(count)));
            }
            sides.push(Cell::List(writer.cells(cells)?));
        }
        writer.cells(sides)
    }

    /// The order of `left` before `right` by a total: larger first, ties by tag.
    fn before(labels: &[Arc<[u8]>], left: (u64, i128), right: (u64, i128)) -> Ordering {
        right
            .1
            .cmp(&left.1)
            .then_with(|| labels[left.0 as usize].cmp(&labels[right.0 as usize]))
    }

    /// Moves the `touched` countries in each ranking to their totals' places; the positions
    /// and ranks that changed go on from `hour`.
    fn rank(&mut self, writer: &mut Writer, hour: i32, touched: &[u64]) -> io::Result<()> {
        for channel in 0..8 {
            let mut moved = Vec::new();
            let mut changed = HashSet::new();
            for &actor in touched {
                let amount = self.countries[actor as usize]
                    .totals
                    .and_then(|totals| totals[channel]);
                if self.rankings[channel].amounts.get(&actor).copied() != amount {
                    changed.insert(actor);
                    if let Some(amount) = amount {
                        moved.push((actor, amount));
                    }
                }
            }
            if changed.is_empty() {
                continue;
            }
            let labels = &self.labels;
            moved.sort_unstable_by(|&left, &right| Self::before(labels, left, right));
            let ranking = &mut self.rankings[channel];
            for &actor in &changed {
                ranking.amounts.remove(&actor);
            }
            for &(actor, amount) in &moved {
                ranking.amounts.insert(actor, amount);
            }
            let old = std::mem::take(&mut ranking.order);
            let mut order = Vec::with_capacity(ranking.amounts.len());
            let mut kept = old
                .iter()
                .copied()
                .filter(|actor| !changed.contains(actor))
                .peekable();
            let mut moved = moved.into_iter().peekable();
            loop {
                let next = match (kept.peek(), moved.peek()) {
                    (Some(&left), Some(&(right, amount))) => {
                        let held = (left, ranking.amounts[&left]);
                        if Self::before(labels, held, (right, amount)) == Ordering::Greater {
                            moved.next().map(|(actor, _)| actor)
                        } else {
                            kept.next()
                        }
                    }
                    (Some(_), None) => kept.next(),
                    (None, Some(_)) => moved.next().map(|(actor, _)| actor),
                    (None, None) => break,
                };
                order.extend(next);
            }
            let mut ranks = HashMap::new();
            for position in 0..old.len().max(order.len()) {
                let (was, now) = (old.get(position).copied(), order.get(position).copied());
                if was == now {
                    continue;
                }
                if let Some(now) = now {
                    ranks.insert(now, Some(position));
                }
                if let Some(was) = was {
                    ranks.entry(was).or_insert(None);
                }
            }
            for position in 0..old.len().max(order.len()) {
                let (was, now) = (old.get(position).copied(), order.get(position).copied());
                if was == now {
                    continue;
                }
                let key = match ranking.positions.get(position) {
                    Some(&key) => key,
                    None => {
                        let key = self.actors.key(
                            writer,
                            Place {
                                actor: FAMILY,
                                measure: POSITIONS + channel as u64,
                                dimension: Some(position as u64),
                            },
                            (RunMode::Exact, 0),
                            Some(DOMAIN),
                        )?;
                        ranking.positions.push(key);
                        key
                    }
                };
                self.carry
                    .value(writer, key, hour, now.map(i128::from), false)?;
            }
            for (actor, position) in ranks {
                let key = self.countries[actor as usize].keys[10 + channel];
                // A country holding the same rank in another place cannot happen: positions
                // that kept their country were skipped.
                self.carry.value(
                    writer,
                    key,
                    hour,
                    position.map(|position| position as i128 + 1),
                    false,
                )?;
            }
            ranking.order = order;
        }
        Ok(())
    }

    fn aid(
        &mut self,
        writer: &mut Writer,
        hour: i32,
        gap: bool,
        projection: aid::Projection,
    ) -> io::Result<()> {
        for (channel, reasons) in projection.reasons.into_iter().enumerate() {
            for (reason, count) in reasons {
                *self.warnings[channel].entry(reason).or_default() += count;
                self.stale = true;
            }
        }
        let reads = self.reads.ok_or_else(invalid)?;
        for (channel, read) in projection.reads.into_iter().enumerate() {
            self.carry
                .value(writer, reads[channel], hour, read.then_some(1), gap)?;
        }
        if let Some(parties) = projection.parties {
            let mut present = BTreeSet::new();
            for party in &parties {
                present.insert(self.party(writer, party, hour)?);
            }
            for &actor in self.present.union(&present) {
                self.carry.value(
                    writer,
                    self.proofs[&actor][3],
                    hour,
                    present.contains(&actor).then_some(1),
                    gap || !self.present.contains(&actor),
                )?;
            }
            for actor in self.present.difference(&present) {
                for channel in 0..3 {
                    self.carry
                        .value(writer, self.proofs[actor][channel], hour, None, false)?;
                }
                self.current.remove(actor);
            }
            self.present = present;
        }
        for sender in projection.senders {
            let actor = self.party(writer, &sender.party, hour)?;
            let old = self.current.remove(&actor);
            let changed = old.as_ref() != Some(&sender);
            for channel in 0..3 {
                let next = &sender.channels[channel];
                let before = old.as_ref().map(|old| &old.channels[channel]);
                if !gap && before == Some(next) {
                    continue;
                }
                self.carry.value(
                    writer,
                    self.proofs[&actor][channel],
                    hour,
                    next.complete.then_some(1),
                    gap,
                )?;
                if changed {
                    for (reason, count) in &next.reasons {
                        *self.warnings[channel].entry(reason.clone()).or_default() += count;
                        self.stale = true;
                    }
                }
                let hosts: BTreeSet<&Party> = next
                    .values
                    .keys()
                    .chain(before.into_iter().flat_map(|before| before.values.keys()))
                    .collect();
                for host in hosts {
                    if !gap
                        && before.is_some_and(|before| {
                            before.complete == next.complete
                                && before.values.get(host) == next.values.get(host)
                        })
                    {
                        continue;
                    }
                    let listed = next.values.contains_key(host);
                    let value = next
                        .complete
                        .then(|| i128::from(next.values.get(host).copied().unwrap_or(0)));
                    let host = self.party(writer, host, hour)?;
                    let link = match self.link_keys.get(&(actor, host)) {
                        Some(&link) => link,
                        None => {
                            let link = self.links.len() as u64;
                            let mut keys = [0; 3];
                            for (at, key) in keys.iter_mut().enumerate() {
                                *key = self.key(
                                    writer,
                                    actor,
                                    AID_VALUE,
                                    Some(link * 3 + at as u64),
                                    0,
                                )?;
                            }
                            self.links.push((actor, host, 0, keys));
                            self.link_keys.insert((actor, host), link);
                            self.changed_links.insert(link);
                            link
                        }
                    };
                    if listed && self.links[link as usize].2 & 1 << channel == 0 {
                        self.links[link as usize].2 |= 1 << channel;
                        self.changed_links.insert(link);
                    }
                    let key = self.links[link as usize].3[channel];
                    self.carry.value(writer, key, hour, value, gap)?;
                }
            }
            if changed && sender.channels[0].complete && sender.channels[2].complete {
                let land = &sender.channels[0].values;
                let tallies = &sender.channels[2].values;
                for host in land.keys().chain(tallies.keys()).collect::<BTreeSet<_>>() {
                    if land.get(host).copied().unwrap_or(0)
                        != tallies.get(host).copied().unwrap_or(0)
                    {
                        *self.warnings[0]
                            .entry(
                                "VolunteerHosts tally differs from resolved volunteer list".into(),
                            )
                            .or_default() += 1;
                        self.stale = true;
                    }
                }
            }
            self.current.insert(actor, sender);
        }
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
            let reasons = |reasons: &BTreeMap<String, usize>| {
                Scalar::List(
                    reasons
                        .iter()
                        .map(|(reason, &count)| {
                            Scalar::List(vec![
                                Scalar::Text(reason.as_bytes().into()),
                                Scalar::Unsigned(count as u64),
                            ])
                        })
                        .collect(),
                )
            };
            let mut scalars = vec![
                (
                    AVAILABLE,
                    Scalar::List(
                        self.available
                            .iter()
                            .map(|&read| Scalar::Boolean(read))
                            .collect(),
                    ),
                ),
                (
                    COUNTS,
                    Scalar::List(vec![
                        Scalar::Unsigned(self.unreadable),
                        Scalar::Unsigned(self.composition),
                    ]),
                ),
                (INVALID, reasons(&self.invalid)),
            ];
            for (channel, warnings) in self.warnings.iter().enumerate() {
                scalars.push((AID_WARNINGS + channel as u64, reasons(warnings)));
            }
            for (id, scalar) in scalars {
                let cell = family::cell(writer, &scalar)?;
                writer.catalogue(id, cell)?;
            }
            self.stale = false;
        }
        for link in std::mem::take(&mut self.changed_links) {
            let (sender, host, channels, _) = self.links[link as usize];
            let cell = family::cell(
                writer,
                &Scalar::List(vec![
                    Scalar::Unsigned(sender),
                    Scalar::Unsigned(host),
                    Scalar::Unsigned(u64::from(channels)),
                ]),
            )?;
            writer.catalogue(LINKS + link, cell)?;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection names its countries anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let countries = self
            .countries
            .iter()
            .enumerate()
            .filter(|(_, country)| country.holder.is_some() || country.version != 0)
            .map(|(actor, country)| SavedCountry {
                actor: actor as u64,
                holder: country.holder,
                totals: country.totals,
                mix: country.mix.as_deref().map(|mix| {
                    mix.clone()
                        .map(|side| side.map(|parts| parts.into_iter().collect()))
                }),
                version: country.version,
            })
            .collect();
        let current = self
            .current
            .iter()
            .map(|(&actor, sender)| SavedSender {
                actor,
                channels: sender.channels.each_ref().map(|channel| SavedChannel {
                    complete: channel.complete,
                    values: channel
                        .values
                        .iter()
                        .map(|(party, &count)| (self.parties[party], count))
                        .collect(),
                    reasons: channel
                        .reasons
                        .iter()
                        .map(|(reason, &count)| (reason.clone(), count))
                        .collect(),
                }),
                hosts: sender.hosts.clone(),
                volunteers: sender.volunteers.clone(),
                expeditionaries: sender.expeditionaries.clone(),
            })
            .collect();
        let saved = Saved {
            last: self.last,
            live: self.live.iter().copied().collect(),
            countries,
            present: self.present.iter().copied().collect(),
            current,
            warnings: self.warnings.clone(),
            available: self.available,
            unreadable: self.unreadable,
            composition: self.composition,
            invalid: self.invalid.clone(),
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
        let count = directory.actors.len();
        emit.labels = directory
            .actors
            .iter()
            .map(|actor| Arc::from(&actor.name[..]))
            .collect();
        emit.countries = vec![Country::default(); count];
        for (actor, described) in directory.actors.iter().enumerate() {
            let actor = actor as u64;
            if let Some((slot, id, tag)) = party_of(&described.name) {
                let mut keys = [0; 4];
                for (channel, key) in keys.iter_mut().enumerate() {
                    *key = directory
                        .key(actor, AID_PROOF, Some(channel as u64))
                        .ok_or_else(invalid)?;
                }
                emit.proofs.insert(actor, keys);
                emit.parties.insert(
                    Party {
                        slot,
                        id,
                        tag: Arc::from(tag),
                    },
                    actor,
                );
                continue;
            }
            let mut keys = Vec::with_capacity(18);
            for measure in (0..10).chain(RANKS..RANKS + 8) {
                keys.push(directory.key(actor, measure, None).ok_or_else(invalid)?);
            }
            emit.countries[actor as usize].keys = keys;
        }
        let mut reads = [0; 3];
        for (channel, key) in reads.iter_mut().enumerate() {
            *key = directory
                .key(FAMILY, AID_READ, Some(channel as u64))
                .ok_or_else(invalid)?;
        }
        emit.reads = Some(reads);
        for (channel, ranking) in emit.rankings.iter_mut().enumerate() {
            ranking.positions = directory
                .dimensions(FAMILY, POSITIONS + channel as u64)
                .iter()
                .enumerate()
                .map(|(at, &(position, key))| {
                    (position == at as u64).then_some(key).ok_or_else(invalid)
                })
                .collect::<io::Result<_>>()?;
        }
        let mut link = 0;
        while let Some(scalar) = directory.scalar(LINKS + link) {
            let [sender, host, channels] = scalar.list() else {
                return Err(invalid());
            };
            let field = |scalar: &Scalar| {
                scalar
                    .integer()
                    .and_then(|value| u64::try_from(value).ok())
                    .ok_or_else(invalid)
            };
            let (sender, host) = (field(sender)?, field(host)?);
            let mut keys = [0; 3];
            for (at, key) in keys.iter_mut().enumerate() {
                *key = directory
                    .key(sender, AID_VALUE, Some(link * 3 + at as u64))
                    .ok_or_else(invalid)?;
            }
            emit.links
                .push((sender, host, field(channels)? as u8, keys));
            emit.link_keys.insert((sender, host), link);
            link += 1;
        }
        let mut id = 0;
        while let Some(scalar) = directory.scalar(NAMES + id) {
            let name =
                std::str::from_utf8(scalar.text().ok_or_else(invalid)?).map_err(|_| invalid())?;
            emit.subunits.insert(name.to_owned(), id);
            id += 1;
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.live = saved.live.into_iter().collect();
        emit.present = saved.present.into_iter().collect();
        emit.warnings = saved.warnings;
        emit.available = saved.available;
        emit.unreadable = saved.unreadable;
        emit.composition = saved.composition;
        emit.invalid = saved.invalid;
        for country in saved.countries {
            let held = emit
                .countries
                .get_mut(country.actor as usize)
                .filter(|held| !held.keys.is_empty())
                .ok_or_else(invalid)?;
            held.holder = country.holder;
            held.totals = country.totals;
            held.mix = country
                .mix
                .map(|mix| Arc::new(mix.map(|side| side.map(|parts| parts.into_iter().collect()))));
            held.version = country.version;
        }
        let by_actor: HashMap<u64, Party> = emit
            .parties
            .iter()
            .map(|(party, &actor)| (actor, party.clone()))
            .collect();
        let party = |actor: u64| by_actor.get(&actor).cloned().ok_or_else(invalid);
        for sender in saved.current {
            let mut channels: [aid::Channel; 3] = Default::default();
            for (channel, saved) in channels.iter_mut().zip(sender.channels) {
                channel.complete = saved.complete;
                for (host, count) in saved.values {
                    channel.values.insert(party(host)?, count);
                }
                channel.reasons = saved.reasons.into_iter().collect();
            }
            let restored = Sender {
                party: party(sender.actor)?,
                channels,
                hosts: sender.hosts,
                volunteers: sender.volunteers,
                expeditionaries: sender.expeditionaries,
            };
            emit.current.insert(sender.actor, restored);
        }
        // Each ranking holds every country with a known total, in rank order.
        for channel in 0..8 {
            let mut known: Vec<(u64, i128)> = emit
                .countries
                .iter()
                .enumerate()
                .filter_map(|(actor, country)| Some((actor as u64, country.totals?[channel]?)))
                .collect();
            let labels = &emit.labels;
            known.sort_unstable_by(|&left, &right| Self::before(labels, left, right));
            let ranking = &mut emit.rankings[channel];
            ranking.order = known.iter().map(|&(actor, _)| actor).collect();
            ranking.amounts = known.into_iter().collect();
        }
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct SavedCountry {
    actor: u64,
    holder: Option<(u32, u32)>,
    totals: Option<Totals>,
    mix: Option<[Option<Vec<(String, i64)>>; 2]>,
    version: u64,
}

#[derive(Serialize, Deserialize)]
struct SavedChannel {
    complete: bool,
    values: Vec<(u64, i64)>,
    reasons: Vec<(String, usize)>,
}

#[derive(Serialize, Deserialize)]
struct SavedSender {
    actor: u64,
    channels: [SavedChannel; 3],
    hosts: Vec<(u32, u32, u32, i32)>,
    volunteers: Vec<(u32, u32, u64)>,
    expeditionaries: Option<Vec<u64>>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    live: Vec<u64>,
    countries: Vec<SavedCountry>,
    present: Vec<u64>,
    current: Vec<SavedSender>,
    warnings: [BTreeMap<String, usize>; 3],
    available: [bool; 5],
    unreadable: u64,
    composition: u64,
    invalid: BTreeMap<String, usize>,
}
