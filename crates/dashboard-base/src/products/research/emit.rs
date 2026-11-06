//! Research's emit: the country step ([`super::Scan`]) carried hour by hour through the projected
//! facts ([`super::facts`]), each country's state published on the family's series as the
//! hourly walk published it. Per tag: its coverage, availability, slot states (each an event at
//! its first hour) and progress (each read an event) as runs; its closed history as events; its
//! open history at each cut; and its grant inventory while a supplying read admits it.
//!
//! Each segment's lens ([`super::lens`]) folds every country on the walk; from a country's first
//! read a day into the segment the emit binds it ([`Lensed`]) and appends what the lens published
//! of it, stepping itself only the slot states the two hold apart, until the lens hands it back.
//!
//! A tag's grants are admitted from a read of its holder's ResearchModifiers at the read's hour
//! while every owner of its slot holding grant rows shares that read, through every hour after
//! it of an unbroken walk with the same holder, short of a read period.

use super::{
    Country, CountryRows, FreshRead, Indexes, Reads, Scan, counts_down,
    facts::{self, Fresh, Grant, Index},
    lens::{self, Bind, Items, Key, Op, Payload, Prior, Publish, Taken},
};
use crate::{
    ResearchCoverage, ResearchDetails, ResearchEntry, ResearchHistory, ResearchProgress,
    ResearchSlotHistory, ResearchSlotState,
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

/// `error`, met while a lens holds countries the emit cannot publish without: the fold is
/// wrong, never the recorded data, so it ends the family rather than refuse its hour.
fn unrefused(error: io::Error) -> io::Error {
    if error.kind() == io::ErrorKind::InvalidData {
        io::Error::other(format!("Research lens: {error}"))
    } else {
        error
    }
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

    /// The handle of `key` where it was catalogued.
    fn held(&self, key: Key) -> Option<KeyHandle> {
        match key {
            Key::Coverage => Some(self.coverage),
            Key::History => Some(self.history),
            Key::Available(slot) => self.slots.get(&slot).map(|keys| keys[0]),
            Key::State(slot) => self.slots.get(&slot).map(|keys| keys[1]),
            Key::Progress(slot) => self.slots.get(&slot).map(|keys| keys[2]),
        }
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

/// What an index step hands the country step: the slots to observe and whether the holders
/// changed, or None for an incomplete hour.
pub(super) fn refreshed(index: &Index) -> Option<(BTreeSet<u32>, bool)> {
    match index {
        Index::Incomplete => None,
        Index::Quiet => Some((BTreeSet::new(), false)),
        Index::Changed(changed) => Some((changed.dirty.clone(), changed.countries_changed)),
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

/// What a lens's values of its first read's hour stand for in the emit's state, by where they
/// stand ([`Prior`]).
#[derive(Default)]
struct Sigma {
    coverage: Option<i32>,
    available: BTreeMap<u32, i32>,
    progress: BTreeMap<u32, i32>,
    /// Each slot's assignment the lens began from, as an assignment's first observed hour.
    refs: BTreeMap<u32, i32>,
    /// Each slot's assignment the lens began from: its first observed hour, the observation it
    /// started after and its continuity.
    entries: BTreeMap<u32, (i32, Option<i32>, i32)>,
    continuity: Option<i32>,
    starts: BTreeMap<u32, i32>,
}

/// An item the emit holds that the lens never did, closed where the lens first holds one of
/// its kind in its slot.
enum Residue {
    Available(ResearchCoverage),
    Entry(ResearchEntry),
    Progress(ResearchProgress),
}

/// A country a lens folds, as the emit bound it.
struct Lensed {
    tag: String,
    canonical: u32,
    virgin: Option<i32>,
    sigma: Sigma,
    /// Each slot state the emit steps itself, where the lens holds its slot's apart.
    chains: BTreeMap<u32, ResearchSlotState>,
    /// The slots the lens never held, and the single items.
    slots: Vec<ResearchSlotHistory>,
    items: BTreeMap<(u32, Items), Residue>,
    /// Its slot reads not joined, at the bind: the emit's, and the lens's.
    unjoined: (usize, usize),
}

/// Where `a` and `b`, a lens's value of `virgin`'s hour standing for one of the emit's, agree:
/// `held` records what it stands for, once.
fn bind_hour(
    held: &mut Option<i32>,
    virgin: Option<i32>,
    first: i32,
    real: i32,
    lens: i32,
) -> Option<()> {
    if virgin == Some(lens) {
        if real >= first || held.is_some_and(|held| held != real) {
            return None;
        }
        *held = Some(real);
        Some(())
    } else {
        (real == lens).then_some(())
    }
}

/// As [`bind_hour`], an hour kept by slot.
fn bind_slot(
    held: &mut BTreeMap<u32, i32>,
    slot: u32,
    virgin: Option<i32>,
    first: i32,
    real: i32,
    lens: i32,
) -> Option<()> {
    let mut one = held.get(&slot).copied();
    bind_hour(&mut one, virgin, first, real, lens)?;
    if let Some(one) = one {
        held.insert(slot, one);
    }
    Some(())
}

/// As [`bind_slot`], for an optional hour.
fn bind_option(
    held: &mut BTreeMap<u32, i32>,
    slot: u32,
    virgin: Option<i32>,
    first: i32,
    real: Option<i32>,
    lens: Option<i32>,
) -> Option<()> {
    match (real, lens) {
        (Some(real), Some(lens)) => bind_slot(held, slot, virgin, first, real, lens),
        (None, None) => Some(()),
        _ => None,
    }
}

/// Whether two states continue alike in all but their countdown.
fn alike(real: &ResearchSlotState, lens: &ResearchSlotState) -> bool {
    real.last_hour == lens.last_hour
        && real.token == lens.token
        && real.speed_multiplier_raw == lens.speed_multiplier_raw
        && real.banked_days_raw == lens.banked_days_raw
        && real.applied_days_raw == lens.applied_days_raw
        && real.ahead_reduction_years_raw == lens.ahead_reduction_years_raw
        && match (&real.details, &lens.details) {
            (None, None) => true,
            (Some(real), Some(lens)) => {
                real.bonuses == lens.bonuses && real.modifiers == lens.modifiers
            }
            _ => false,
        }
}

/// Whether `entry`, the emit's, matches the lens's `lens` in all its in-segment fields.
fn same_entry(real: &ResearchEntry, lens: &ResearchEntry) -> bool {
    real.token == lens.token
        && real.technology_handle == lens.technology_handle
        && real.database_index == lens.database_index
        && real.level == lens.level
        && real.last_observed_hour == lens.last_observed_hour
        && real.ended_by_hour == lens.ended_by_hour
        && real.outcome_observed_hour == lens.outcome_observed_hour
        && real.outcome == lens.outcome
}

/// How the emit's `real` state of a country binds a lens's `lens` one, where the two fold alike
/// from here on; None where they would not. The lens began at `first`, from the country's read
/// at `virgin`.
fn bound(real: &Country, lens: &Country, first: i32, virgin: Option<i32>) -> Option<Lensed> {
    if real.carry_until != lens.carry_until
        || real.identity != lens.identity
        || real.last_read != lens.last_read
        || real.read_after != lens.read_after
        || real.observed_read != lens.observed_read
        || real.active != lens.active
        || real.entry_occurrences != lens.entry_occurrences
        || real.pending != lens.pending
        || real.samples.len() != lens.samples.len()
    {
        return None;
    }
    // Later reads look a day back alone, past the segment's first hour.
    for (slot, real) in &real.samples {
        let lens = lens.samples.get(slot)?;
        if !real
            .iter()
            .filter(|(hour, _)| *hour >= first)
            .eq(lens.iter().filter(|(hour, _)| *hour >= first))
        {
            return None;
        }
    }
    let mut sigma = Sigma::default();
    bind_hour(
        &mut sigma.continuity,
        virgin,
        first,
        real.continuity_start_hour,
        lens.continuity_start_hour,
    )?;
    if real.unjoined_starts.len() != lens.unjoined_starts.len() {
        return None;
    }
    for (slot, &held) in &lens.unjoined_starts {
        let &real = real.unjoined_starts.get(slot)?;
        bind_slot(&mut sigma.starts, *slot, virgin, first, real, held)?;
    }
    match (
        real.history.coverage.as_slice(),
        lens.history.coverage.as_slice(),
    ) {
        ([], []) => {}
        ([real], [lens]) if real.last_hour == lens.last_hour => {
            bind_hour(&mut sigma.coverage, virgin, first, real.first_hour, lens.first_hour)?;
        }
        _ => return None,
    }
    let mut chains = BTreeMap::new();
    let mut items = BTreeMap::new();
    let mut slots = Vec::new();
    for real_slot in &real.history.slots {
        let index = real_slot.index;
        let Some(lens_slot) = lens.history.slots.iter().find(|slot| slot.index == index) else {
            // A slot the lens never held: nothing of it is read again unless the slot is.
            if real_slot
                .entries
                .iter()
                .any(|entry| entry.ended_by_hour.is_none())
                || real_slot.availability.iter().any(|span| span.last_hour >= first)
                || real_slot.progress.iter().any(|progress| progress.last_hour >= first)
                || real_slot.states.iter().any(|state| state.last_hour >= first)
            {
                return None;
            }
            slots.push(real_slot.clone());
            continue;
        };
        match (
            real_slot.availability.as_slice(),
            lens_slot.availability.as_slice(),
        ) {
            ([], []) => {}
            ([real], [lens]) if real.last_hour == lens.last_hour => {
                bind_slot(
                    &mut sigma.available,
                    index,
                    virgin,
                    first,
                    real.first_hour,
                    lens.first_hour,
                )?;
            }
            _ => return None,
        }
        match (real_slot.states.as_slice(), lens_slot.states.as_slice()) {
            ([], []) => {}
            ([real], [lens]) if alike(real, lens) => {
                bind_option(
                    &mut sigma.refs,
                    index,
                    virgin,
                    first,
                    real.entry_first_observed_hour,
                    lens.entry_first_observed_hour,
                )?;
                let rd = |state: &ResearchSlotState| {
                    state.details.as_ref().map(|details| details.remaining_days)
                };
                if real.first_hour != lens.first_hour || rd(real) != rd(lens) {
                    chains.insert(index, real.clone());
                }
            }
            _ => return None,
        }
        match (real_slot.progress.as_slice(), lens_slot.progress.as_slice()) {
            ([], []) => {}
            ([real], [lens])
                if real.last_hour == lens.last_hour
                    && real.base_days_raw == lens.base_days_raw
                    && real.progress_raw == lens.progress_raw =>
            {
                bind_slot(
                    &mut sigma.progress,
                    index,
                    virgin,
                    first,
                    real.first_hour,
                    lens.first_hour,
                )?;
                bind_option(
                    &mut sigma.refs,
                    index,
                    virgin,
                    first,
                    real.entry_first_observed_hour,
                    lens.entry_first_observed_hour,
                )?;
            }
            ([real], []) if real.last_hour < first => {
                items.insert((index, Items::Progress), Residue::Progress(real.clone()));
            }
            _ => return None,
        }
        match (real_slot.entries.as_slice(), lens_slot.entries.as_slice()) {
            ([real], []) if real.ended_by_hour.is_some() => {
                items.insert((index, Items::Entries), Residue::Entry(real.clone()));
            }
            (reals, lenses) if reals.len() == lenses.len() => {
                for (real, held) in reals.iter().zip(lenses) {
                    if !same_entry(real, held) {
                        return None;
                    }
                    if virgin == Some(held.first_observed_hour) {
                        if real.first_observed_hour >= first || sigma.entries.contains_key(&index)
                        {
                            return None;
                        }
                        sigma.entries.insert(
                            index,
                            (
                                real.first_observed_hour,
                                real.started_after_hour,
                                real.continuity_start_hour,
                            ),
                        );
                        bind_slot(
                            &mut sigma.refs,
                            index,
                            virgin,
                            first,
                            real.first_observed_hour,
                            held.first_observed_hour,
                        )?;
                    } else {
                        if real.first_observed_hour != held.first_observed_hour
                            || real.started_after_hour != held.started_after_hour
                        {
                            return None;
                        }
                        bind_hour(
                            &mut sigma.continuity,
                            virgin,
                            first,
                            real.continuity_start_hour,
                            held.continuity_start_hour,
                        )?;
                    }
                }
            }
            _ => return None,
        }
    }
    if lens
        .history
        .slots
        .iter()
        .any(|lens| !real.history.slots.iter().any(|real| real.index == lens.index))
    {
        return None;
    }
    // A progress the emit holds alone is never shown again: no assignment of its slot is its.
    for ((index, _), item) in &items {
        let Residue::Progress(progress) = item else {
            continue;
        };
        let Some(slot) = real.history.slots.iter().find(|slot| slot.index == *index) else {
            continue;
        };
        if progress.entry_first_observed_hour.is_some_and(|hour| {
            slot.entries.iter().any(|entry| entry.first_observed_hour == hour)
                || slot
                    .states
                    .iter()
                    .any(|state| state.entry_first_observed_hour == Some(hour))
        }) {
            return None;
        }
    }
    Some(Lensed {
        tag: String::new(),
        canonical: 0,
        virgin,
        sigma,
        chains,
        slots,
        items,
        unjoined: (real.unjoined_reads, lens.unjoined_reads),
    })
}

/// An hour field an event's text names by its key, wherever it stands in the text.
#[derive(Clone, Copy)]
enum PriorHour {
    /// `firstHour`.
    First,
    /// `entryFirstObservedHour`.
    Entry,
    /// `continuityStartHour`.
    Continuity,
}

/// Sets every `field` of `event` holding `virgin`'s hour to `value`.
fn map_prior(
    event: &mut lens::Event,
    field: PriorHour,
    virgin: i32,
    value: Option<i32>,
) -> io::Result<()> {
    let set = |hour: &mut i32| -> io::Result<()> {
        if *hour == virgin {
            *hour =
                value.ok_or_else(|| io::Error::other("A research lens names a prior hour unbound"))?;
        }
        Ok(())
    };
    let set_some = |hour: &mut Option<i32>| match hour {
        Some(hour) => set(hour),
        None => Ok(()),
    };
    let progress = |progress: &mut ResearchProgress| match field {
        PriorHour::First => set(&mut progress.first_hour),
        PriorHour::Entry => set_some(&mut progress.entry_first_observed_hour),
        PriorHour::Continuity => Ok(()),
    };
    match event {
        lens::Event::State(state) => match field {
            PriorHour::First => set(&mut state.first_hour),
            PriorHour::Entry => set_some(&mut state.entry_first_observed_hour),
            PriorHour::Continuity => Ok(()),
        },
        lens::Event::Read(read) => read.as_mut().map_or(Ok(()), progress),
        lens::Event::Closed(closed) => match (&mut **closed, field) {
            (Closed::Coverage { span } | Closed::Available { span, .. }, PriorHour::First) => {
                set(&mut span.first_hour)
            }
            (Closed::Progress { progress: held, .. }, _) => progress(held),
            (Closed::Entry { entry, .. }, PriorHour::Continuity) => {
                set(&mut entry.continuity_start_hour)
            }
            _ => Ok(()),
        },
    }
}

/// The fresh reads `index` kept.
fn fresh_of(index: &Index) -> &BTreeMap<u32, Fresh> {
    static NONE: BTreeMap<u32, Fresh> = BTreeMap::new();
    match index {
        Index::Changed(changed) => &changed.fresh,
        _ => &NONE,
    }
}

impl Lensed {
    /// The lens's `entry` of slot `slot`, as the emit holds it.
    fn entry(&self, slot: u32, mut entry: ResearchEntry) -> ResearchEntry {
        let Some(virgin) = self.virgin else {
            return entry;
        };
        if entry.first_observed_hour == virgin
            && let Some(&(first, after, continuity)) = self.sigma.entries.get(&slot)
        {
            entry.first_observed_hour = first;
            entry.started_after_hour = after;
            entry.continuity_start_hour = continuity;
        } else if entry.continuity_start_hour == virgin
            && let Some(continuity) = self.sigma.continuity
        {
            entry.continuity_start_hour = continuity;
        }
        entry
    }

    /// The lens's `country` as the emit holds it, its first read's hour mapped back and what
    /// the emit held alone put back.
    fn taken(&mut self, mut country: Country) -> Country {
        country.chains = None;
        country.unjoined_reads = self.unjoined.0 + country.unjoined_reads.saturating_sub(self.unjoined.1);
        if let Some(virgin) = self.virgin {
            let sigma = &self.sigma;
            let map = |hour: &mut i32, real: Option<i32>| {
                if *hour == virgin
                    && let Some(real) = real
                {
                    *hour = real;
                }
            };
            for span in &mut country.history.coverage {
                map(&mut span.first_hour, sigma.coverage);
            }
            map(&mut country.continuity_start_hour, sigma.continuity);
            for (slot, start) in &mut country.unjoined_starts {
                map(start, sigma.starts.get(slot).copied());
            }
            for slot in &mut country.history.slots {
                let index = slot.index;
                let refs = sigma.refs.get(&index).copied();
                for span in &mut slot.availability {
                    map(&mut span.first_hour, sigma.available.get(&index).copied());
                }
                for progress in &mut slot.progress {
                    map(&mut progress.first_hour, sigma.progress.get(&index).copied());
                    if let Some(hour) = &mut progress.entry_first_observed_hour {
                        map(hour, refs);
                    }
                }
                for state in &mut slot.states {
                    if let Some(hour) = &mut state.entry_first_observed_hour {
                        map(hour, refs);
                    }
                }
            }
            for slot in &mut country.history.slots {
                let index = slot.index;
                let entries = std::mem::take(&mut slot.entries);
                slot.entries = entries
                    .into_iter()
                    .map(|entry| self.entry(index, entry))
                    .collect();
            }
        }
        for (index, chain) in std::mem::take(&mut self.chains) {
            if let Some(slot) = country.history.slots.iter_mut().find(|slot| slot.index == index)
                && let Some(last) = slot.states.last_mut()
            {
                let last_hour = last.last_hour;
                *last = chain;
                last.last_hour = last_hour;
            }
        }
        for ((index, kind), item) in std::mem::take(&mut self.items) {
            let Some(slot) = country.history.slots.iter_mut().find(|slot| slot.index == index)
            else {
                continue;
            };
            match (kind, item) {
                (Items::Availability, Residue::Available(span)) => slot.availability.insert(0, span),
                (Items::Entries, Residue::Entry(entry)) => slot.entries.insert(0, entry),
                (Items::Progress, Residue::Progress(progress)) => slot.progress.insert(0, progress),
                _ => {}
            }
        }
        country.history.slots.append(&mut self.slots);
        country
    }
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
    /// The countries the walk's lens folds, by the number it names them by; None for one it
    /// bound that the emit does not.
    lensed: HashMap<u32, Option<Lensed>>,
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
            lensed: HashMap::new(),
            last: None,
            first: None,
        }
    }

    /// Keeps one projected hour, or what the lens hands back with the hour kept last.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("research.emit", None);
        let at = hour.hour;
        if hour.handover {
            if Some(at) != self.last {
                return Err(io::Error::other(
                    "A research lens hands back an hour the emit did not keep",
                ));
            }
            if let Some(part) = hour.lens {
                self.returned(part.returned).map_err(unrefused)?;
            }
            return Ok(());
        }
        match self.last {
            Some(last) if last.checked_add(1) == Some(at) => {}
            Some(last) if at <= last => {
                return Err(crate::invalid(
                    "A research hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every run, every admitted read and the known hours.
            Some(last) => {
                if !self.lensed.is_empty() {
                    return Err(io::Error::other(
                        "A research lens did not hand its countries back",
                    ));
                }
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
        // Only the walk's read of an hour refuses it while a lens holds countries: the lens
        // hands them back through the hour before. An hour the emit itself would refuse then
        // could not publish them, so it ends the family instead.
        match self.keep(writer, at, hour) {
            Err(error) if !self.lensed.is_empty() => Err(unrefused(error)),
            kept => kept,
        }
    }

    /// Keeps hour `at`: its country step, what it publishes, and what the lens hands on with it.
    fn keep(&mut self, writer: &mut Writer, at: i32, hour: facts::Hour) -> io::Result<()> {
        let facts::Hour {
            mut index,
            grants,
            lens,
            ..
        } = hour;
        self.step(at, &mut index)?;
        self.publish(writer, at)?;
        if let Some(part) = lens {
            let lens::Part {
                ops,
                returned,
                bound,
            } = *part;
            let fresh = fresh_of(&index);
            (|| -> io::Result<()> {
                for ops in ops {
                    self.replay(writer, at, ops, fresh)?;
                }
                self.returned(returned)?;
                for bind in bound {
                    self.bind(bind);
                }
                Ok(())
            })()
            .map_err(unrefused)?;
        }
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

    /// Binds the country `bind` names where the lens's state of it folds as the emit's does.
    fn bind(&mut self, bind: Bind) {
        let Bind {
            number,
            tag,
            canonical,
            first,
            virgin,
            country,
            recheck,
            held,
        } = bind;
        let lensed = self.scan.countries.get(&tag).and_then(|real| {
            let mut lensed = bound(real, &country, first, virgin)?;
            if recheck != self.scan.recheck.contains(&canonical) {
                return None;
            }
            // What the carry holds of its keys is what the lens's does.
            let held: HashMap<Key, i128> = held.into_iter().collect();
            let keys = self.tags.get(&tag);
            let carried = |key: Key| keys.and_then(|keys| keys.held(key)).and_then(|key| self.carry.held(key));
            if carried(Key::Coverage) != held.get(&Key::Coverage).copied() {
                return None;
            }
            for slot in &real.history.slots {
                let index = slot.index;
                for key in [Key::Available(index), Key::Progress(index)] {
                    if carried(key) != held.get(&key).copied() {
                        return None;
                    }
                }
                let state = carried(Key::State(index));
                let lens = held.get(&Key::State(index)).copied();
                match lensed.chains.get(&index) {
                    Some(chain) => {
                        let shown = lens.map(|_| i128::from(chain.first_hour));
                        if state != shown {
                            return None;
                        }
                    }
                    None if state != lens => return None,
                    None => {}
                }
            }
            lensed.tag.clone_from(&tag);
            lensed.canonical = canonical;
            Some(lensed)
        });
        if lensed.is_some() {
            self.scan.lens(&tag, canonical);
        }
        self.lensed.insert(number, lensed);
    }

    /// Takes back the countries the lens returns.
    fn returned(&mut self, returned: Vec<Taken>) -> io::Result<()> {
        for Taken {
            number,
            country,
            recheck,
        } in returned
        {
            let lensed = self
                .lensed
                .remove(&number)
                .ok_or_else(|| io::Error::other("A research lens returns a country it never bound"))?;
            if let Some(mut lensed) = lensed {
                let country = lensed.taken(*country);
                let tag = std::mem::take(&mut lensed.tag);
                self.scan.unlens(&tag, country, lensed.canonical, recheck);
                if !self.stale.contains(&tag) {
                    self.stale.insert(tag);
                }
            }
        }
        Ok(())
    }

    /// Appends what a bound country published at `at`.
    fn replay(
        &mut self,
        writer: &mut Writer,
        at: i32,
        ops: lens::Ops,
        fresh: &BTreeMap<u32, Fresh>,
    ) -> io::Result<()> {
        let Some(Some(lensed)) = self.lensed.get_mut(&ops.country) else {
            return Ok(());
        };
        let keys = tag_keys(&mut self.actors, &mut self.tags, writer, &lensed.tag, at)?;
        for op in ops.ops {
            match op {
                Op::Chain(chain) => {
                    let Some(state) = lensed.chains.get_mut(&chain.slot) else {
                        continue;
                    };
                    let kept = chain.gate
                        && match &state.details {
                            Some(details) => counts_down(
                                details.remaining_days,
                                state.first_hour,
                                at,
                                chain.remaining,
                            ),
                            None => true,
                        };
                    match (kept, chain.kept) {
                        (true, _) => state.last_hour = at,
                        // Both start the read's state: they hold it alike from here.
                        (false, false) => {
                            lensed.chains.remove(&chain.slot);
                        }
                        (false, true) => {
                            let mut read = started(fresh, lensed.canonical, &chain, at)?;
                            if let (Some(virgin), Some(hour)) =
                                (lensed.virgin, read.entry_first_observed_hour)
                                && hour == virgin
                            {
                                read.entry_first_observed_hour =
                                    Some(*lensed.sigma.refs.get(&chain.slot).ok_or_else(|| {
                                        io::Error::other("A research lens names a prior assignment unbound")
                                    })?);
                            }
                            let [_, key, _] = slot_keys(&mut self.actors, keys, writer, chain.slot)?;
                            self.carry.value(writer, key, at, Some(i128::from(at)), true)?;
                            event(writer, key, at, kind::STATE, &read)?;
                            *state = read;
                        }
                    }
                }
                Op::Slot(slot) => {
                    slot_keys(&mut self.actors, keys, writer, slot)?;
                    if let Some(at_slot) = lensed.slots.iter().position(|held| held.index == slot) {
                        let held = lensed.slots.remove(at_slot);
                        if let Some(span) = held.availability.into_iter().last() {
                            lensed.items.insert((slot, Items::Availability), Residue::Available(span));
                        }
                        if let Some(entry) = held.entries.into_iter().last() {
                            lensed.items.insert((slot, Items::Entries), Residue::Entry(entry));
                        }
                        if let Some(progress) = held.progress.into_iter().last() {
                            lensed.items.insert((slot, Items::Progress), Residue::Progress(progress));
                        }
                    }
                }
                Op::Filled(slot, items) => {
                    let Some(item) = lensed.items.remove(&(slot, items)) else {
                        continue;
                    };
                    let closed = match item {
                        Residue::Available(span) => Closed::Available { slot, span },
                        Residue::Entry(entry) => Closed::Entry { slot, entry },
                        Residue::Progress(progress) => Closed::Progress { slot, progress },
                    };
                    event(writer, keys.history, at, kind::CLOSED, &closed)?;
                }
                Op::Read => {
                    for slot in &lensed.slots {
                        let [_, _, key] = slot_keys(&mut self.actors, keys, writer, slot.index)?;
                        event(writer, key, at, kind::READ, &None::<&ResearchProgress>)?;
                    }
                }
                Op::Value {
                    key,
                    mut value,
                    mut starts,
                } => {
                    if let Key::State(slot) = key
                        && let Some(state) = lensed.chains.get(&slot)
                    {
                        value = value.map(|_| i128::from(state.first_hour));
                        starts = false;
                    }
                    let handle = keys.handle(&mut self.actors, writer, key)?;
                    self.carry.value(writer, handle, at, value, starts)?;
                }
                Op::Event {
                    key,
                    event: mut published,
                    prior,
                } => {
                    if let Key::State(slot) = key
                        && lensed.chains.contains_key(&slot)
                    {
                        continue;
                    }
                    let handle = keys.handle(&mut self.actors, writer, key)?;
                    if let (Some(prior), Some(virgin)) = (prior, lensed.virgin) {
                        let sigma = &lensed.sigma;
                        let pairs: Vec<(PriorHour, Option<i32>)> = match prior {
                            Prior::Coverage => vec![(PriorHour::First, sigma.coverage)],
                            Prior::Available(slot) => {
                                vec![(PriorHour::First, sigma.available.get(&slot).copied())]
                            }
                            Prior::Progress(slot) => vec![
                                (PriorHour::First, sigma.progress.get(&slot).copied()),
                                (PriorHour::Entry, sigma.refs.get(&slot).copied()),
                            ],
                            Prior::Assignment(slot) => {
                                vec![(PriorHour::Entry, sigma.refs.get(&slot).copied())]
                            }
                            Prior::Continuity => vec![(PriorHour::Continuity, sigma.continuity)],
                        };
                        for (field, value) in pairs {
                            map_prior(&mut published, field, virgin, value)?;
                        }
                    }
                    match &published {
                        lens::Event::State(state) => {
                            event(writer, handle, at, kind::STATE, &**state)?
                        }
                        lens::Event::Read(read) => event(writer, handle, at, kind::READ, read)?,
                        lens::Event::Closed(closed) => {
                            event(writer, handle, at, kind::CLOSED, &**closed)?
                        }
                    }
                }
                Op::Entry { slot, entry } => {
                    let entry = lensed.entry(slot, *entry);
                    event(writer, keys.history, at, kind::CLOSED, &Closed::Entry { slot, entry })?;
                }
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
        if !self.lensed.is_empty() {
            return Err(io::Error::other(
                "A research lens did not hand its countries back",
            ));
        }
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

/// The state slot `chain.slot` of `canonical`'s read starts at `at`, which the lens's read went on
/// past: as the read's rows give it.
fn started(
    fresh: &BTreeMap<u32, Fresh>,
    canonical: u32,
    chain: &lens::Chain,
    at: i32,
) -> io::Result<ResearchSlotState> {
    let missing = || io::Error::other("A research lens's read was not kept");
    let kept = fresh.get(&canonical).ok_or_else(missing)?;
    let position = chain.slot as usize;
    let input = kept.inputs.get(position).ok_or_else(missing)?;
    let derived = kept.derived.get(position).ok_or_else(missing)?;
    let token = chain
        .held
        .then(|| std::str::from_utf8(&input.token).map(str::to_owned))
        .transpose()
        .map_err(|_| io::Error::other("A research technology token is not text"))?;
    Ok(ResearchSlotState {
        first_hour: at,
        last_hour: at,
        token,
        entry_first_observed_hour: chain.entry,
        speed_multiplier_raw: input.research_speed_raw.to_string(),
        banked_days_raw: input.banked_days_raw.to_string(),
        applied_days_raw: input.applied_days_raw.to_string(),
        ahead_reduction_years_raw: chain.held.then(|| input.ahead_reduction_raw.to_string()),
        details: Some(ResearchDetails {
            remaining_days: chain.remaining,
            bonuses: derived.bonuses.as_deref().cloned(),
            modifiers: derived.modifiers.as_deref().cloned(),
        }),
    })
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
