//! World tension's emit: each read's world tension, contribution sum, unnamed part and read
//! hour as runs on the family's series publication, each contributor's contribution and share
//! of world tension, the hours a read holds, the threat sources as events, and the reads'
//! counts.
//!
//! A read is an hour that read the tables while Countries, CountryLinks, WorldTension and
//! ThreatCountryAggregates are complete and WorldTension holds its row; its values hold over
//! the following hours until the next read, an hour without the tables or an hour not walked,
//! which ends every run. A read after an hour without one starts a segment. A contributor is
//! its canonical slot with the tag the slot last named, numbered as first read: its actor.
//! Countries and links are remembered from every hour recording them, for the whole branch.

use super::facts::{self, Identity, Sources, Tension};
use super::contribution_share;
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use engine_stability::cworld_threat;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, sync::Arc};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// The family's keys: world tension in contribution units, the contributions' sum, its part no
/// recorded country names, the hour of the read holding each hour, zero wherever a read holds,
/// and the threat sources' events.
pub const WORLD: u64 = 0;
pub const SUM: u64 = 1;
pub const UNNAMED: u64 = 2;
pub const READ: u64 = 3;
pub const ZERO: u64 = 4;
pub const EVENTS: u64 = 5;
const MEASURES: [u64; 6] = [WORLD, SUM, UNNAMED, READ, ZERO, EVENTS];
/// A contributor's contribution, and its share of world tension, in contribution units.
pub const VALUE: u64 = 0;
pub const SHARE: u64 = 1;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// Family scalar: `[reads, reads with an unnamed contribution, reads of partial sources,
/// reads whose aggregates disagree with their sources, reads whose tension is not the cap of
/// the sum]`.
pub const COUNTS: u64 = 0;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;
/// Run values are in contribution units, 100,000 a percentage point.
pub const DECIMALS: u8 = 5;

/// Event kinds, on the family's EVENTS key, each naming its event by id.
pub mod kind {
    /// `[Unsigned id, Integer date, Text reason, Text tag | Null, Integer added]`: first read.
    pub const SOURCE: u64 = 0;
    /// `[Unsigned id, Integer added]`: what it adds changed.
    pub const ADDED: u64 = 1;
    /// `[Unsigned id, Text tag]`: its country was named.
    pub const NAMED: u64 = 2;
    /// `[Unsigned id, Integer last read]`: it is no longer listed.
    pub const DROPPED: u64 = 3;
    /// `[Unsigned id]`: it is listed again.
    pub const RESUMED: u64 = 4;
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved tension emit is not valid")
}

/// A contributor's actor name: its canonical slot, then its tag.
pub fn actor_name(slot: u32, tag: &str) -> Vec<u8> {
    let mut name = slot.to_le_bytes().to_vec();
    name.extend_from_slice(tag.as_bytes());
    name
}

/// The canonical slot and tag an actor name holds.
pub fn contributor(name: &[u8]) -> Option<(u32, &str)> {
    let (slot, tag) = name.split_first_chunk::<4>()?;
    Some((u32::from_le_bytes(*slot), std::str::from_utf8(tag).ok()?))
}

/// An event's identity: its recorded country and second country references, date, reason, and
/// its place among sources equal in all four within a read.
type EventKey = (u32, u32, i32, Box<[u8]>, u32);

/// What an event holds now.
#[derive(Clone, Serialize, Deserialize)]
struct Held {
    key: EventKey,
    named: bool,
    added: i64,
}

/// A contributor: its keys, and its contribution at the read held.
#[derive(Clone, Copy, Default)]
struct Entry {
    keys: [KeyHandle; 2],
    value: i64,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    family: Option<[KeyHandle; 6]>,
    entries: Vec<Entry>,
    by_country: HashMap<(u32, Arc<str>), u64>,
    /// Each slot's tag and each raw link's slot, as last recorded.
    known: HashMap<u32, Arc<str>>,
    known_links: HashMap<u32, u32>,
    /// The inputs as last recorded.
    identity: bool,
    tension: Option<Option<Tension>>,
    aggregates: Option<Arc<[(u32, i64)]>>,
    sources: Option<Sources>,
    /// Inputs that changed since the read last took them.
    contributions_stale: bool,
    sources_stale: bool,
    /// Whether a read holds the last hour, and the last read.
    open: bool,
    last_read: Option<i32>,
    /// The held read's sum and unnamed part, and the contributors it holds.
    sum: i64,
    unnamed: i64,
    current: Vec<u64>,
    /// The values the held read published, to skip a read changing none.
    published: Option<(i64, i64, i64)>,
    events: Vec<Held>,
    indices: HashMap<EventKey, u64>,
    /// The events the last listing listed; each event's last listing, and the listings made.
    active: Vec<u64>,
    listed: Vec<u64>,
    listing: u64,
    counts: [u64; 5],
    stale: bool,
    last: Option<i32>,
    /// The first hour since the last cut.
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
            family: None,
            entries: Vec::new(),
            by_country: HashMap::new(),
            known: HashMap::new(),
            known_links: HashMap::new(),
            identity: false,
            tension: None,
            aggregates: None,
            sources: None,
            contributions_stale: true,
            sources_stale: true,
            open: false,
            last_read: None,
            sum: 0,
            unnamed: 0,
            current: Vec::new(),
            published: None,
            events: Vec::new(),
            indices: HashMap::new(),
            active: Vec::new(),
            listed: Vec::new(),
            listing: 0,
            counts: [0; 5],
            stale: true,
            last: None,
            first: None,
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, DECIMALS), Some(DOMAIN))
    }

    fn family(&mut self, writer: &mut Writer<'_>) -> io::Result<[KeyHandle; 6]> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let mut keys = [0; 6];
        for (key, measure) in keys.iter_mut().zip(MEASURES) {
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

    /// The contributor of `slot` named `tag`, numbered at `hour` when it is new.
    fn entry(&mut self, writer: &mut Writer<'_>, slot: u32, tag: &Arc<str>, hour: i32) -> io::Result<u64> {
        if let Some(&actor) = self.by_country.get(&(slot, Arc::clone(tag))) {
            return Ok(actor);
        }
        let actor = self.actors.actor(writer, &actor_name(slot, tag), hour)?;
        let mut keys = [0; 2];
        for (key, measure) in keys.iter_mut().zip([VALUE, SHARE]) {
            *key = self.key(
                writer,
                Place {
                    actor,
                    measure,
                    dimension: None,
                },
            )?;
        }
        if actor as usize != self.entries.len() {
            return Err(io::Error::other("Tension contributors are numbered out of order"));
        }
        self.entries.push(Entry { keys, value: 0 });
        self.by_country.insert((slot, Arc::clone(tag)), actor);
        Ok(actor)
    }

    /// `key`'s value from `at` on: a run starts a segment where `starts`, or where it held none
    /// the hour before.
    fn set(
        &mut self,
        writer: &mut Writer<'_>,
        key: KeyHandle,
        at: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()> {
        let starts = value.is_some() && (starts || self.carry.held(key).is_none());
        self.carry.value(writer, key, at, value, starts)?;
        Ok(())
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("tension.emit", None);
        let at = hour.hour;
        match self.last {
            Some(last) if last.checked_add(1) == Some(at) => {}
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "A tension hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every run.
            Some(last) => {
                self.carry.close(writer, last)?;
                self.open = false;
            }
            None => {}
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        self.family(writer)?;
        let mut identified = false;
        if let Some(facts) = hour.facts {
            let facts::Facts {
                identity,
                tension,
                aggregates,
                sources,
                ..
            } = *facts;
            if let Some(identity) = identity {
                identified = true;
                self.identify(identity.as_deref());
            }
            if let Some(tension) = tension {
                self.tension = tension;
            }
            if let Some(aggregates) = aggregates {
                self.aggregates = aggregates;
                self.contributions_stale = true;
            }
            if let Some(sources) = sources {
                self.sources_stale |= sources.as_ref().is_none_or(|sources| sources.regrouped);
                self.sources = sources;
            }
        }
        let present = self.identity && self.aggregates.is_some() && self.tension.is_some();
        let tension = match self.tension {
            Some(Some(tension)) if present => tension,
            // A read of an empty table ends the held one, as an hour without the tables does.
            Some(None) if present && !hour.read => return Ok(()),
            _ => return self.end(writer, at),
        };
        if !hour.read {
            return Ok(());
        }
        let starts = !self.open;
        if identified || starts || self.contributions_stale {
            self.contribute(writer, at)?;
        }
        self.count(tension)?;
        if identified || starts || self.sources_stale {
            self.list(writer, at)?;
        }
        self.publish(writer, at, tension, starts)?;
        self.open = true;
        self.last_read = Some(at);
        Ok(())
    }

    /// Remembers the countries the hour records; none while they are not complete.
    fn identify(&mut self, identity: Option<&Identity>) {
        self.identity = identity.is_some();
        if let Some(identity) = identity {
            for (slot, tag) in &identity.tags {
                self.known.insert(*slot, Arc::clone(tag));
            }
            self.known_links.extend(identity.links.iter().copied());
            self.contributions_stale = true;
            self.sources_stale = true;
        }
    }

    /// The read's contributions: every nonzero aggregate, by slot, named by the tag its slot last
    /// recorded; the rest is unnamed.
    fn contribute(&mut self, writer: &mut Writer<'_>, at: i32) -> io::Result<()> {
        let aggregates = self.aggregates.clone().unwrap_or_else(|| Arc::from([]));
        let (mut sum, mut unnamed) = (0_i64, 0_i64);
        for &actor in &self.current {
            self.entries[actor as usize].value = 0;
        }
        let mut current = Vec::with_capacity(aggregates.len());
        for &(slot, total) in aggregates.iter() {
            sum = sum.checked_add(total).ok_or_else(|| {
                io::Error::other("World tension contribution sum exceeds signed 64-bit range")
            })?;
            let Some(tag) = self.known.get(&slot).cloned() else {
                unnamed = unnamed.checked_add(total).ok_or_else(|| {
                    io::Error::other(
                        "World tension unnamed contribution sum exceeds signed 64-bit range",
                    )
                })?;
                continue;
            };
            let actor = self.entry(writer, slot, &tag, at)?;
            self.entries[actor as usize].value = total;
            current.push(actor);
        }
        // The contributors that left are published at zero, then dropped by `publish`.
        self.current.extend(current);
        self.current.sort_unstable();
        self.current.dedup();
        self.sum = sum;
        self.unnamed = unnamed;
        self.contributions_stale = false;
        self.published = None;
        Ok(())
    }

    /// The read's counts, as the row scan counts them.
    fn count(&mut self, tension: Tension) -> io::Result<()> {
        self.counts[0] += 1;
        self.counts[1] += u64::from(self.unnamed != 0);
        let whole = self.sources.as_ref().is_some_and(|sources| {
            sources.complete && usize::try_from(tension.source_count).ok() == Some(sources.rows.len())
        });
        if !whole {
            self.counts[2] += 1;
        } else if let Some(sources) = &self.sources {
            let current = sources.sum.ok_or_else(|| {
                io::Error::other("World tension source sum exceeds signed 64-bit range")
            })?;
            self.counts[3] += u64::from(current != self.sum);
        }
        // World tension stores the clamped and scaled sum.
        let capped = self.sum.clamp(0, cworld_threat::CLAMP_CEILING) / cworld_threat::SCALAR_DIVISOR;
        self.counts[4] += u64::from(capped != tension.total_raw);
        self.stale = true;
        Ok(())
    }

    /// The read's threat sources: each listed one's event, first read, changed or listed again,
    /// and each one listed before and no longer.
    fn list(&mut self, writer: &mut Writer<'_>, at: i32) -> io::Result<()> {
        let [.., events] = self.family(writer)?;
        self.listing += 1;
        let listing = self.listing;
        let sources = self.sources.as_ref().map(|sources| Arc::clone(&sources.rows));
        let rows = sources.as_deref().unwrap_or_default();
        let mut active = Vec::with_capacity(rows.len());
        // Sources equal in all four, counted in stored order.
        let mut seen: HashMap<(u32, u32, i32, &[u8]), u32> = HashMap::new();
        for row in rows {
            let place = seen
                .entry((row.country, row.second_country, row.date, &row.reason[..]))
                .or_default();
            let key: EventKey = (
                row.country,
                row.second_country,
                row.date,
                row.reason.clone(),
                *place,
            );
            *place += 1;
            let tag = self
                .known_links
                .get(&row.country)
                .and_then(|slot| self.known.get(slot))
                .cloned();
            let id = match self.indices.get(&key) {
                Some(&id) => {
                    if self.listed[id as usize] + 1 != listing {
                        let fields = writer.cells([Cell::Unsigned(id)])?;
                        self.event(writer, events, at, kind::RESUMED, fields)?;
                    }
                    let held = &mut self.events[id as usize];
                    if !held.named
                        && let Some(tag) = &tag
                    {
                        held.named = true;
                        let text = family::cell(writer, &Scalar::Text(tag.as_bytes().into()))?;
                        let fields = writer.cells([Cell::Unsigned(id), text])?;
                        self.event(writer, events, at, kind::NAMED, fields)?;
                    }
                    let held = &mut self.events[id as usize];
                    if held.added != row.final_raw {
                        held.added = row.final_raw;
                        let fields =
                            writer.cells([Cell::Unsigned(id), Cell::Integer(i128::from(row.final_raw))])?;
                        self.event(writer, events, at, kind::ADDED, fields)?;
                    }
                    id
                }
                None => {
                    let id = self.events.len() as u64;
                    let reason = dashboard_base::catalogue::display_text(&row.reason).map_err(|reason| {
                        io::Error::other(format!(
                            "The recorded threat reason at hour {} {reason}",
                            row.date
                        ))
                    })?;
                    let reason = family::cell(writer, &Scalar::Text(reason.as_bytes().into()))?;
                    let named = tag.is_some();
                    let tag = match &tag {
                        Some(tag) => family::cell(writer, &Scalar::Text(tag.as_bytes().into()))?,
                        None => Cell::Null,
                    };
                    let fields = writer.cells([
                        Cell::Unsigned(id),
                        Cell::Integer(i128::from(row.date)),
                        reason,
                        tag,
                        Cell::Integer(i128::from(row.final_raw)),
                    ])?;
                    self.event(writer, events, at, kind::SOURCE, fields)?;
                    self.events.push(Held {
                        key: key.clone(),
                        named,
                        added: row.final_raw,
                    });
                    self.listed.push(0);
                    self.indices.insert(key, id);
                    id
                }
            };
            self.listed[id as usize] = listing;
            active.push(id);
        }
        let last_read = self.last_read;
        for id in std::mem::replace(&mut self.active, active) {
            if self.listed[id as usize] != listing {
                let last = last_read.ok_or_else(|| io::Error::other("A listed threat source was never read"))?;
                let fields = writer.cells([Cell::Unsigned(id), Cell::Integer(i128::from(last))])?;
                self.event(writer, events, at, kind::DROPPED, fields)?;
            }
        }
        self.sources_stale = false;
        Ok(())
    }

    fn event(
        &mut self,
        writer: &mut Writer<'_>,
        key: KeyHandle,
        at: i32,
        kind: u64,
        fields: dashboard_series::Blob,
    ) -> io::Result<()> {
        let sequence = writer.sequence()?;
        writer.event(Event {
            key,
            native_hour: at,
            read_hour: Some(at),
            sequence,
            kind,
            fields,
        })
    }

    /// The read's values from `at` on: every key starts again where `starts`.
    fn publish(&mut self, writer: &mut Writer<'_>, at: i32, tension: Tension, starts: bool) -> io::Result<()> {
        let [world, sum, unnamed, read, zero, _] = self.family(writer)?;
        let scaled = i128::from(tension.total_raw) * i128::from(cworld_threat::SCALAR_DIVISOR);
        for (key, value) in [
            (world, scaled),
            (sum, i128::from(self.sum)),
            (unnamed, i128::from(self.unnamed)),
            (read, i128::from(at)),
            (zero, 0),
        ] {
            self.set(writer, key, at, Some(value), starts)?;
        }
        let published = (tension.total_raw, self.sum, self.unnamed);
        if !starts && self.published == Some(published) {
            return Ok(());
        }
        let actors: Vec<u64> = if starts {
            (0..self.entries.len() as u64).collect()
        } else {
            self.current.clone()
        };
        for actor in actors {
            let entry = self.entries[actor as usize];
            let value = i128::from(entry.value);
            let share = contribution_share(value, i128::from(tension.total_raw), i128::from(self.sum))?;
            self.set(writer, entry.keys[0], at, Some(value), starts)?;
            self.set(writer, entry.keys[1], at, Some(share), starts)?;
        }
        self.current.retain(|&actor| self.entries[actor as usize].value != 0);
        self.published = Some(published);
        Ok(())
    }

    /// Ends the read held before `at`.
    fn end(&mut self, writer: &mut Writer<'_>, at: i32) -> io::Result<()> {
        if !self.open {
            return Ok(());
        }
        let family = self.family(writer)?;
        for key in &family[..5] {
            self.set(writer, *key, at, None, false)?;
        }
        for at_entry in 0..self.entries.len() {
            let keys = self.entries[at_entry].keys;
            for key in keys {
                self.set(writer, key, at, None, false)?;
            }
        }
        self.open = false;
        self.published = None;
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the counts where they changed.
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
            let counts = Scalar::List(self.counts.iter().map(|&count| Scalar::Unsigned(count)).collect());
            let cell = family::cell(writer, &counts)?;
            writer.catalogue(COUNTS, cell)?;
            self.stale = false;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            known: self.known.iter().map(|(&slot, tag)| (slot, Arc::clone(tag))).collect(),
            known_links: self.known_links.iter().map(|(&raw, &slot)| (raw, slot)).collect(),
            identity: self.identity,
            // A complete table holding no row is not the table missing.
            tension: self.tension.map(|tension| [tension]),
            aggregates: self.aggregates.clone(),
            sources: self.sources.clone(),
            contributions_stale: self.contributions_stale,
            sources_stale: self.sources_stale,
            open: self.open,
            last_read: self.last_read,
            sum: self.sum,
            unnamed: self.unnamed,
            values: self
                .entries
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.value != 0)
                .map(|(actor, entry)| (actor as u64, entry.value))
                .collect(),
            current: self.current.clone(),
            published: self.published,
            events: self.events.clone(),
            active: self.active.clone(),
            counts: self.counts,
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
        emit.stale = false;
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let mut family = [0; 6];
        let mut found = 0;
        for (key, measure) in family.iter_mut().zip(MEASURES) {
            if let Some(held) = directory.key(FAMILY, measure, None) {
                *key = held;
                found += 1;
            }
        }
        emit.family = match found {
            0 => None,
            6 => Some(family),
            _ => return Err(invalid()),
        };
        for (actor, held) in directory.actors.iter().enumerate() {
            let (slot, tag) = contributor(&held.name).ok_or_else(invalid)?;
            let mut keys = [0; 2];
            for (key, measure) in keys.iter_mut().zip([VALUE, SHARE]) {
                *key = directory
                    .key(actor as u64, measure, None)
                    .ok_or_else(invalid)?;
            }
            emit.entries.push(Entry { keys, value: 0 });
            emit.by_country.insert((slot, Arc::from(tag)), actor as u64);
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.known = saved.known.into_iter().collect();
        emit.known_links = saved.known_links.into_iter().collect();
        emit.identity = saved.identity;
        emit.tension = saved.tension.map(|[tension]| tension);
        emit.aggregates = saved.aggregates;
        emit.sources = saved.sources;
        emit.contributions_stale = saved.contributions_stale;
        emit.sources_stale = saved.sources_stale;
        emit.open = saved.open;
        emit.last_read = saved.last_read;
        emit.sum = saved.sum;
        emit.unnamed = saved.unnamed;
        for (actor, value) in saved.values {
            emit.entries
                .get_mut(actor as usize)
                .ok_or_else(invalid)?
                .value = value;
        }
        if saved.current.iter().any(|&actor| actor as usize >= emit.entries.len()) {
            return Err(invalid());
        }
        emit.current = saved.current;
        emit.published = saved.published;
        emit.listing = 1;
        emit.listed = vec![0; saved.events.len()];
        for &id in &saved.active {
            *emit.listed.get_mut(id as usize).ok_or_else(invalid)? = 1;
        }
        for (id, held) in saved.events.iter().enumerate() {
            emit.indices.insert(held.key.clone(), id as u64);
        }
        emit.events = saved.events;
        emit.active = saved.active;
        emit.counts = saved.counts;
        emit.last = saved.last;
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    known: Vec<(u32, Arc<str>)>,
    known_links: Vec<(u32, u32)>,
    identity: bool,
    tension: Option<[Option<Tension>; 1]>,
    aggregates: Option<Arc<[(u32, i64)]>>,
    sources: Option<Sources>,
    contributions_stale: bool,
    sources_stale: bool,
    open: bool,
    last_read: Option<i32>,
    sum: i64,
    unnamed: i64,
    values: Vec<(u64, i64)>,
    current: Vec<u64>,
    published: Option<(i64, i64, i64)>,
    events: Vec<Held>,
    active: Vec<u64>,
    counts: [u64; 5],
    last: Option<i32>,
}
