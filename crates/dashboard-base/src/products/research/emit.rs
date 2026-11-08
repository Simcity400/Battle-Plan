//! Research's emit: the country step ([`super::Scan`]) carried hour by hour through the projected
//! facts ([`super::facts`]), each country's state published on the family's series as the
//! hourly walk published it. Per tag: its coverage, availability, slot states (each an event at
//! its first hour) and progress (each read an event) as runs; its closed history as events; its
//! open history at each cut; and its grant inventory while a supplying read admits it.
//!
//! A tag's grants are admitted from a read of its holder's ResearchModifiers at the read's hour
//! while every owner of its slot holding grant rows shares that read, through every hour after
//! it of an unbroken walk with the same holder, short of a read period.

use super::{
    Country, CountryRows, FreshRead, Indexes, Reads, Scan,
    facts::{self, Fresh, Grant, Index},
};
use crate::{
    ResearchCoverage, ResearchEntry, ResearchHistory, ResearchProgress, ResearchSlotState,
};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{Actors, Directory, Place},
};
use replay_data::cadence::STAGGER_PERIOD;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A tag's coverage; its closed history as events; its open history, an event at each cut; and
/// its grant inventory's version while a supplying read admits it, each version an event.
pub const COVERAGE: u64 = 0;
pub const HISTORY: u64 = 1;
pub const TAIL: u64 = 2;
pub const GRANTS: u64 = 3;
/// A tag's slots, by slot index: availability; the state's first hour, each state an event
/// there; and progress, each read an event.
pub const AVAILABLE: u64 = 4;
pub const STATE: u64 = 5;
pub const PROGRESS: u64 = 6;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// Event kinds, each `[Text json]`.
pub mod kind {
    /// A closed history item ([`super::Closed`]), on a tag's HISTORY key.
    pub const CLOSED: u64 = 0;
    /// The open history ([`super::Open`]), on a tag's TAIL key.
    pub const OPEN: u64 = 1;
    /// A slot state, on its STATE key.
    pub const STATE: u64 = 2;
    /// A read's progress, or null, on its PROGRESS key.
    pub const READ: u64 = 3;
    /// A grant inventory ([`crate::products::research::facts::Content`]), on a tag's GRANTS key.
    pub const GRANT: u64 = 4;
}

/// A key of a country's series.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, replay_spool::Spooled)]
pub(super) enum Key {
    Coverage,
    History,
    Available(u32),
    State(u32),
    Progress(u32),
}

/// An event's text, as a country's publish hands it.
pub(super) enum Payload<'a> {
    State(&'a ResearchSlotState),
    Read(Option<&'a ResearchProgress>),
    Closed(&'a Closed),
}

/// Where a country's publish goes.
pub(super) trait Publish {
    /// Slot `slot` is published.
    fn slot(&mut self, slot: u32) -> io::Result<()>;
    fn value(&mut self, key: Key, at: i32, value: Option<i128>, starts: bool) -> io::Result<()>;
    fn event(&mut self, key: Key, at: i32, payload: Payload<'_>) -> io::Result<()>;
    /// The country's read was observed at the hour.
    fn read(&mut self) -> io::Result<()>;
}

/// A history item no later hour changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, replay_spool::Spooled)]
#[serde(tag = "kind")]
pub(super) enum Closed {
    Coverage {
        span: ResearchCoverage,
    },
    Available {
        slot: u32,
        span: ResearchCoverage,
    },
    Progress {
        slot: u32,
        progress: ResearchProgress,
    },
    Entry {
        slot: u32,
        entry: ResearchEntry,
    },
}

/// The history items later hours can still change, and the slot reads not joined.
#[derive(Serialize, Deserialize)]
pub(super) struct Open {
    pub(super) history: ResearchHistory,
    pub(super) unjoined: usize,
}

/// [`Open`] as an emit writes it, borrowing the history.
#[derive(Serialize)]
struct OpenRef<'a> {
    history: &'a ResearchHistory,
    unjoined: usize,
}

fn invalid() -> io::Error {
    crate::invalid("Saved research emit is not valid")
}

/// `error`, met by the country step: the family's refusal at its hour.
fn refused(error: io::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

/// `text`, an event's JSON, as one event of `key` at `hour`.
fn put(writer: &mut Writer, key: KeyHandle, hour: i32, kind: u64, text: &[u8]) -> io::Result<()> {
    let text = writer.blob(&mut &text[..], text.len() as u64)?;
    let fields = writer.cells([Cell::Text(text)])?;
    let sequence = writer.sequence()?;
    writer.event(Event {
        key,
        native_hour: hour,
        read_hour: None,
        sequence,
        kind,
        fields,
    })
}

/// `value` as one event of `key` at `hour`.
fn event(
    writer: &mut Writer,
    key: KeyHandle,
    hour: i32,
    kind: u64,
    value: &impl Serialize,
) -> io::Result<()> {
    put(writer, key, hour, kind, &serde_json::to_vec(value)?)
}

/// Every key's evidence from `first` through `last`: each hour walked is known.
fn known(writer: &mut Writer, first: i32, last: i32) -> io::Result<()> {
    writer
        .domain(
            DOMAIN,
            Evidence {
                span: Span::new(first, last)?,
                known: Knownness::Known,
                supplying_read: None,
                holder: None,
                identity_first: None,
            },
        )
        .map(drop)
}

/// A tag's keys.
#[derive(Clone, Default)]
struct Keys {
    coverage: KeyHandle,
    history: KeyHandle,
    tail: KeyHandle,
    grants: KeyHandle,
    /// AVAILABLE, STATE and PROGRESS of each slot.
    slots: BTreeMap<u32, [KeyHandle; 3]>,
    actor: u64,
}

impl Keys {
    /// The handle of `key`, its slot's keys catalogued when new.
    fn handle(
        &mut self,
        actors: &mut Actors,
        writer: &mut Writer,
        key: Key,
    ) -> io::Result<KeyHandle> {
        Ok(match key {
            Key::Coverage => self.coverage,
            Key::History => self.history,
            Key::Available(slot) => slot_keys(actors, self, writer, slot)?[0],
            Key::State(slot) => slot_keys(actors, self, writer, slot)?[1],
            Key::Progress(slot) => slot_keys(actors, self, writer, slot)?[2],
        })
    }

}

fn key(
    actors: &mut Actors,
    writer: &mut Writer,
    actor: u64,
    measure: u64,
    dimension: Option<u64>,
) -> io::Result<KeyHandle> {
    actors.key(
        writer,
        Place {
            actor,
            measure,
            dimension,
        },
        (RunMode::Exact, 0),
        Some(DOMAIN),
    )
}

/// `tag`'s keys, catalogued at `hour` when it is new.
fn tag_keys<'k>(
    actors: &mut Actors,
    tags: &'k mut HashMap<String, Keys>,
    writer: &mut Writer,
    tag: &str,
    hour: i32,
) -> io::Result<&'k mut Keys> {
    if !tags.contains_key(tag) {
        let actor = actors.actor(writer, tag.as_bytes(), hour)?;
        let mut keys = Keys {
            actor,
            ..Keys::default()
        };
        for (held, measure) in [
            (&mut keys.coverage, COVERAGE),
            (&mut keys.history, HISTORY),
            (&mut keys.tail, TAIL),
            (&mut keys.grants, GRANTS),
        ] {
            *held = key(actors, writer, actor, measure, None)?;
        }
        tags.insert(tag.to_owned(), keys);
    }
    Ok(tags.get_mut(tag).expect("catalogued research tag"))
}

/// The keys of `keys`' tag's slot `slot`.
fn slot_keys(
    actors: &mut Actors,
    keys: &mut Keys,
    writer: &mut Writer,
    slot: u32,
) -> io::Result<[KeyHandle; 3]> {
    if let Some(&held) = keys.slots.get(&slot) {
        return Ok(held);
    }
    let mut held = [0; 3];
    for (handle, measure) in held.iter_mut().zip([AVAILABLE, STATE, PROGRESS]) {
        *handle = key(actors, writer, keys.actor, measure, Some(u64::from(slot)))?;
    }
    keys.slots.insert(slot, held);
    Ok(held)
}

/// An hour's rows as the projection kept them, with the read hours of the indexes.
pub(super) struct KeptReads<'a> {
    pub(super) dates: &'a BTreeMap<u32, Vec<(u32, i32)>>,
    pub(super) fresh: &'a BTreeMap<u32, Fresh>,
}

impl Reads for KeptReads<'_> {
    fn read_hour(&self, slot: u32, owner: u32) -> Option<i32> {
        self.dates
            .get(&slot)?
            .iter()
            .find(|(held, _)| *held == owner)
            .map(|(_, read)| *read)
    }

    fn fresh(&self, slot: u32, _: &[u32]) -> io::Result<FreshRead<'_>> {
        let kept = self
            .fresh
            .get(&slot)
            .ok_or_else(|| io::Error::other("A fresh research read was not kept"))?;
        Ok(FreshRead::kept(kept))
    }
}

/// The indexes the country step reads beside its scan, as the projection's index steps leave
/// them.
#[derive(Default)]
struct Indexed {
    slots: Option<Arc<HashMap<u32, u32>>>,
    rows: CountryRows,
    owners: BTreeMap<u32, Vec<u32>>,
    dates: BTreeMap<u32, Vec<(u32, i32)>>,
}

impl Indexed {
    /// Takes what `index` changed of the indexes out of it.
    fn adopt(&mut self, index: &mut Index) {
        let Index::Changed(changed) = index else {
            return;
        };
        let changes = std::mem::take(&mut changed.changes);
        if let Some((slots, rows)) = changes.countries {
            self.slots = Some(slots);
            self.rows = rows;
        }
        if let Some(owners) = changes.owners {
            self.owners = owners;
        }
        if let Some(whole) = changes.dates {
            self.dates = whole;
        }
        for (slot, dates) in changes.slot_dates {
            match dates {
                Some(dates) => {
                    self.dates.insert(slot, dates);
                }
                None => {
                    self.dates.remove(&slot);
                }
            }
        }
    }

    /// The country step of hour `at`, from its index step taken up, whose slots to observe it
    /// takes, and its fresh reads.
    fn observe(&self, scan: &mut Scan, at: i32, index: &mut Index) -> io::Result<()> {
        static NONE: BTreeMap<u32, Fresh> = BTreeMap::new();
        let (refreshed, fresh) = match index {
            Index::Incomplete => (None, &NONE),
            Index::Quiet => (Some((BTreeSet::new(), false)), &NONE),
            Index::Changed(changed) => (
                Some((
                    std::mem::take(&mut changed.dirty),
                    changed.countries_changed,
                )),
                &changed.fresh,
            ),
        };
        let reads = KeptReads {
            dates: &self.dates,
            fresh,
        };
        let indexes = Indexes {
            slots: self.slots.as_deref(),
            rows: &self.rows,
            owners: &self.owners,
            dates: &self.dates,
        };
        scan.observe_refreshed(at, refreshed, indexes, &reads)
    }
}

/// Publishes what hour `at` changed of `country`, as the hourly walk did; whether it showed
/// progress at its read hour alone, so the next hour observes the country again.
pub(super) fn publish(country: &mut Country, at: i32, out: &mut impl Publish) -> io::Result<bool> {
    country.materialize(at);
    let covered = country.history.coverage.last();
    out.value(
        Key::Coverage,
        at,
        covered.filter(|span| span.last_hour == at).map(|_| 1),
        covered.is_some_and(|span| span.first_hour == at),
    )?;
    let mut read_only = false;
    for slot in &country.history.slots {
        let index = slot.index;
        out.slot(index)?;
        let span = slot.availability.last().filter(|span| span.last_hour == at);
        out.value(
            Key::Available(index),
            at,
            span.map(|_| 1),
            span.is_some_and(|span| span.first_hour == at),
        )?;
        let state = slot.states.last().filter(|state| state.last_hour == at);
        out.value(
            Key::State(index),
            at,
            state.map(|state| i128::from(state.first_hour)),
            state.is_some_and(|state| state.first_hour == at),
        )?;
        if let Some(state) = state.filter(|state| state.first_hour == at) {
            out.event(Key::State(index), at, Payload::State(state))?;
        }
        let current = |progress: &ResearchProgress| {
            state.is_some_and(|state| {
                state.token.is_some()
                    && state.entry_first_observed_hour == progress.entry_first_observed_hour
            })
        };
        let progress = slot
            .progress
            .last()
            .filter(|progress| progress.last_hour == at || current(progress));
        read_only |= progress.is_some_and(|progress| !current(progress));
        let value = progress
            .map(|progress| progress.progress_raw.parse::<i128>())
            .transpose()
            .map_err(|_| crate::invalid("Invalid research progress"))?;
        out.value(
            Key::Progress(index),
            at,
            value,
            progress.is_some_and(|progress| progress.first_hour == at),
        )?;
        // A fresh read dates slot availability even without progress.
        if country.last_read == Some(at) {
            let read = progress.filter(|progress| progress.last_hour == at);
            out.event(Key::Progress(index), at, Payload::Read(read))?;
        }
    }
    if country.last_read == Some(at) {
        out.read()?;
    }
    retain_open(country, |closed| {
        out.event(Key::History, at, Payload::Closed(&closed))
    })?;
    Ok(read_only)
}

/// A country's publish into the family's series.
struct Written<'a> {
    actors: &'a mut Actors,
    keys: &'a mut Keys,
    writer: &'a mut Writer,
    carry: &'a mut Carry,
}

impl Publish for Written<'_> {
    fn slot(&mut self, slot: u32) -> io::Result<()> {
        slot_keys(self.actors, self.keys, self.writer, slot).map(drop)
    }

    fn value(&mut self, key: Key, at: i32, value: Option<i128>, starts: bool) -> io::Result<()> {
        let handle = self.keys.handle(self.actors, self.writer, key)?;
        self.carry
            .value(self.writer, handle, at, value, starts)
            .map(drop)
    }

    fn event(&mut self, key: Key, at: i32, payload: Payload<'_>) -> io::Result<()> {
        let handle = self.keys.handle(self.actors, self.writer, key)?;
        match payload {
            Payload::State(state) => event(self.writer, handle, at, kind::STATE, state),
            Payload::Read(read) => event(self.writer, handle, at, kind::READ, &read),
            Payload::Closed(closed) => event(self.writer, handle, at, kind::CLOSED, closed),
        }
    }

    fn read(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A tag's grant facts and what the emit made of them.
#[derive(Clone, Serialize, Deserialize)]
struct Held {
    grant: Grant,
    /// Its inventory's version, raised at each content it reads.
    version: u64,
    admitted: bool,
}

/// Every tag's grants.
#[derive(Default, Serialize, Deserialize)]
struct Granted {
    /// Whether ResearchModifiers holds every row.
    complete: bool,
    tags: BTreeMap<String, Held>,
    /// The hours a tag's admission is decided again although no fact of it changes: its read's,
    /// when the read was dated past its hour, and the end of its read period.
    deadlines: BTreeSet<(i32, String)>,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    scan: Scan,
    tags: HashMap<String, Keys>,
    indexed: Indexed,
    granted: Granted,
    /// The tags whose open history may have changed since the last cut.
    stale: BTreeSet<String>,
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
            scan: Scan::default(),
            tags: HashMap::new(),
            indexed: Indexed::default(),
            granted: Granted::default(),
            stale: BTreeSet::new(),
            last: None,
            first: None,
        }
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("research.emit", None);
        let at = hour.hour;
        match self.last {
            Some(last) if last.checked_add(1) == Some(at) => {}
            Some(last) if at <= last => {
                return Err(crate::invalid(
                    "A research hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every run, every admitted read and the known hours.
            Some(last) => {
                self.carry.close(writer, last)?;
                if let Some(first) = self.first.take() {
                    known(writer, first, last)?;
                }
                for held in self.granted.tags.values_mut() {
                    held.admitted = false;
                }
            }
            None => {}
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        self.keep(writer, at, hour)
    }

    /// Keeps hour `at`: its country step, and what it publishes.
    fn keep(&mut self, writer: &mut Writer, at: i32, hour: facts::Hour) -> io::Result<()> {
        let facts::Hour {
            mut index, grants, ..
        } = hour;
        self.step(at, &mut index)?;
        self.publish(writer, at)?;
        self.grants(writer, at, grants)
    }

    /// The country step of hour `at`, from its index step.
    fn step(&mut self, at: i32, index: &mut Index) -> io::Result<()> {
        self.indexed.adopt(index);
        self.indexed
            .observe(&mut self.scan, at, index)
            .map_err(refused)
    }

    /// Publishes what hour `at` changed of every country it observed, as the hourly walk did.
    fn publish(&mut self, writer: &mut Writer, at: i32) -> io::Result<()> {
        for tag in &self.scan.observed {
            let country = self
                .scan
                .countries
                .get_mut(tag)
                .expect("observed research country");
            let keys = tag_keys(&mut self.actors, &mut self.tags, writer, tag, at)?;
            let mut out = Written {
                actors: &mut self.actors,
                keys,
                writer: &mut *writer,
                carry: &mut self.carry,
            };
            let read_only = publish(country, at, &mut out)?;
            if read_only && let Some((_, canonical)) = country.identity {
                self.scan.recheck.insert(canonical);
            }
            if !self.stale.contains(tag) {
                self.stale.insert(tag.clone());
            }
        }
        Ok(())
    }

    /// The grants of the tags whose facts changed at `at`, or whose admission is due again.
    fn grants(
        &mut self,
        writer: &mut Writer,
        at: i32,
        update: Option<Box<facts::Grants>>,
    ) -> io::Result<()> {
        let mut decided: BTreeSet<String> = BTreeSet::new();
        if let Some(update) = update {
            let facts::Grants {
                complete,
                whole,
                facts,
            } = *update;
            if let Some(complete) = complete {
                self.granted.complete = complete;
                decided.extend(self.granted.tags.keys().cloned());
            }
            // A tag a whole update leaves out has left the countries: it holds no grants.
            if whole {
                let named: BTreeSet<&str> = facts.iter().map(|(tag, _)| tag.as_str()).collect();
                for (tag, held) in &mut self.granted.tags {
                    if !named.contains(tag.as_str()) {
                        held.grant = Grant::NONE;
                        held.admitted = false;
                        decided.insert(tag.clone());
                    }
                }
            }
            for (tag, grant) in facts {
                let held = self.granted.tags.entry(tag.clone()).or_insert_with(|| Held {
                    grant: Grant::NONE,
                    version: 0,
                    admitted: false,
                });
                // Another holder is another country's read.
                if held.grant.identity != grant.identity {
                    held.admitted = false;
                }
                if grant.content.is_some()
                    && !facts::same_content(held.grant.content.as_ref(), grant.content.as_ref())
                {
                    held.version += 1;
                    let keys = tag_keys(&mut self.actors, &mut self.tags, writer, &tag, at)?;
                    let content = grant.content.as_deref().expect("read grant content");
                    event(writer, keys.grants, at, kind::GRANT, content)?;
                }
                held.grant = grant;
                decided.insert(tag);
            }
        }
        while let Some((due, _)) = self.granted.deadlines.first()
            && *due <= at
        {
            let (_, tag) = self.granted.deadlines.pop_first().expect("due grant read");
            decided.insert(tag);
        }
        for tag in decided {
            let Some(held) = self.granted.tags.get_mut(&tag) else {
                continue;
            };
            let grant = &held.grant;
            let read = grant.read;
            let readable = self.granted.complete
                && grant.identity.is_some()
                && grant.matched
                && grant.content.is_some()
                && read.is_some_and(|read| {
                    read <= at && i64::from(at) - i64::from(read) < i64::from(STAGGER_PERIOD)
                });
            let admitted = readable && (read == Some(at) || held.admitted);
            held.admitted = admitted;
            if let Some(read) = read {
                if read > at {
                    self.granted.deadlines.insert((read, tag.clone()));
                } else if admitted && let Some(end) = read.checked_add(STAGGER_PERIOD) {
                    self.granted.deadlines.insert((end, tag.clone()));
                }
            }
            let version = held.version;
            let keys = tag_keys(&mut self.actors, &mut self.tags, writer, &tag, at)?;
            self.carry.value(
                writer,
                keys.grants,
                at,
                admitted.then_some(i128::from(version)),
                false,
            )?;
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next: each tag's open history that may have changed, and the hours since the last cut as
    /// known.
    pub fn cut(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        // A carried read holds its coverage through the last hour walked.
        self.scan.seal();
        for (_, tag) in &self.scan.deadlines {
            if !self.stale.contains(tag) {
                self.stale.insert(tag.clone());
            }
        }
        for tag in std::mem::take(&mut self.stale) {
            let Some(country) = self.scan.countries.get(&tag) else {
                continue;
            };
            let keys = tag_keys(&mut self.actors, &mut self.tags, writer, &tag, last)?;
            let open = OpenRef {
                history: &country.history,
                unjoined: country.unjoined_reads,
            };
            event(writer, keys.tail, last, kind::OPEN, &open)?;
        }
        self.carry.cut(writer, last)?;
        if let Some(first) = self.first.take() {
            known(writer, first, last)?;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection indexes every holder, link and read hour anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let json = serde_json::to_vec(&SavedRef {
            scan: &self.scan,
            granted: &self.granted,
            last: self.last,
        })?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.actors = Actors::resume(SCHEMA, directory)?;
        for (actor, named) in directory.actors.iter().enumerate() {
            let actor = actor as u64;
            let tag = String::from_utf8(named.name.to_vec()).map_err(|_| invalid())?;
            let mut keys = Keys {
                actor,
                ..Keys::default()
            };
            for (held, measure) in [
                (&mut keys.coverage, COVERAGE),
                (&mut keys.history, HISTORY),
                (&mut keys.tail, TAIL),
                (&mut keys.grants, GRANTS),
            ] {
                *held = directory.key(actor, measure, None).ok_or_else(invalid)?;
            }
            let [available, states, progress] =
                [AVAILABLE, STATE, PROGRESS].map(|measure| directory.dimensions(actor, measure));
            if available.len() != states.len() || available.len() != progress.len() {
                return Err(invalid());
            }
            for ((&(slot, available), &(state_slot, state)), &(progress_slot, progress)) in
                available.iter().zip(states).zip(progress)
            {
                if slot != state_slot || slot != progress_slot {
                    return Err(invalid());
                }
                let slot = u32::try_from(slot).map_err(|_| invalid())?;
                keys.slots.insert(slot, [available, state, progress]);
            }
            emit.tags.insert(tag, keys);
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.scan = saved.scan;
        emit.granted = saved.granted;
        emit.last = saved.last;
        Ok(emit)
    }
}

#[derive(Serialize)]
struct SavedRef<'a> {
    scan: &'a Scan,
    granted: &'a Granted,
    last: Option<i32>,
}

#[derive(Deserialize)]
struct Saved {
    scan: Scan,
    granted: Granted,
    last: Option<i32>,
}

/// The country step needs its latest segment, last assignment and any assignment still
/// awaiting a same-day completion read. Everything else no later hour changes and leaves the
/// state, each item an event.
fn retain_open(
    country: &mut Country,
    mut closed: impl FnMut(Closed) -> io::Result<()>,
) -> io::Result<()> {
    let keep_last = |length: usize| length.saturating_sub(1);
    let remove = keep_last(country.history.coverage.len());
    for span in country.history.coverage.drain(..remove) {
        closed(Closed::Coverage { span })?;
    }
    for slot in &mut country.history.slots {
        // Its last entry stays, and so would one awaiting a completion read.
        if slot.entries.len() > 1 {
            close_entries(slot, &mut country.pending, &mut closed)?;
        }
        let remove = keep_last(slot.availability.len());
        for span in slot.availability.drain(..remove) {
            closed(Closed::Available {
                slot: slot.index,
                span,
            })?;
        }
        // Each state is an event of its own key at its first hour.
        let remove = keep_last(slot.states.len());
        slot.states.drain(..remove);
        let remove = keep_last(slot.progress.len());
        for progress in slot.progress.drain(..remove) {
            closed(Closed::Progress {
                slot: slot.index,
                progress,
            })?;
        }
    }
    Ok(())
}

/// Closes `slot`'s entries but its last and those `pending` awaits, renumbering `pending`.
fn close_entries(
    slot: &mut crate::ResearchSlotHistory,
    pending: &mut [(u32, usize, u32, i32, i32)],
    closed: &mut impl FnMut(Closed) -> io::Result<()>,
) -> io::Result<()> {
    let mut kept = Vec::new();
    let last = slot.entries.len().saturating_sub(1);
    let mut positions = BTreeMap::new();
    for (index, value) in std::mem::take(&mut slot.entries).into_iter().enumerate() {
        if index == last
            || pending
                .iter()
                .any(|pending| pending.0 == slot.index && pending.1 == index)
        {
            positions.insert(index, kept.len());
            kept.push(value);
        } else {
            closed(Closed::Entry {
                slot: slot.index,
                entry: value,
            })?;
        }
    }
    for pending in pending.iter_mut() {
        if pending.0 == slot.index {
            pending.1 = positions[&pending.1];
        }
    }
    slot.entries = kept;
    Ok(())
}
