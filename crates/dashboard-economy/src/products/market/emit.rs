//! The market's emit: every value the observer reads as runs on the family's series
//! publication, and each occurrence as an event in its kind's stream and its parties' indexes.
//!
//! Every [`Field`] is one key, its actor named by the field, interned in the order the observer
//! first reads fields, as the row preparation numbered them. A state value holds from the hour
//! a row shows it to the hour none does; a table not complete, or the countries' identities not
//! read, ends its values; an hour not walked ends every value, and the next observes each again.
//! Each occurrence is kept once, at the read that supplied it, in chunks of at most [`CHUNK`].
//! The family's scalars hold each refusal with its observed hours, the most hours an
//! occurrence lagged its read, the hours whose identities were read, each holder's tags and
//! each named variant's naming reads.

use super::{
    CHUNK, Field, MARKET_EVENTS,
    facts::{self, Events, Occurrence, Refusal, Row, Table},
};
use dashboard_base::products::equipment::{VariantCatalogue, VariantVersion};
use dashboard_series::{
    Blob, Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
    hash::FastMap,
};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashSet},
    io,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 2;
/// Each key's measure: its actor's field.
pub const VALUE: u64 = 0;
/// Family scalars: the refusals (`[[Text reason, Unsigned hours]…]`), the lag (`Integer`) and
/// the hours whose identities were read (`[[Integer first, Integer last]…]`); each holder's tags
/// (`[Unsigned slot, [[Integer hour, Text tag]…]]`, by holder ordinal) and each named variant's
/// naming reads (`[Unsigned variant, Text versions as JSON]`, by variant ordinal).
pub const REFUSALS: u64 = 0;
pub const LAG: u64 = 1;
pub const IDENTIFIED: u64 = 2;
pub const TAGS: u64 = 1 << 24;
pub const VARIANTS: u64 = 1 << 28;
/// Every key's evidence: each walked hour is known.
const DOMAIN: u64 = 0;

/// Event kinds.
pub mod kind {
    /// On a stream chunk: `[Unsigned sequence, Integer hour, Unsigned id, Unsigned occurrence,
    /// Unsigned kind, Unsigned mode, Unsigned agreement, Unsigned seller | Null,
    /// Unsigned buyer | Null, Unsigned variant, Integer amount, Integer total, Unsigned ordinal]`.
    pub const OCCURRENCE: u64 = 0;
    /// On an index chunk: `[Unsigned sequence, Integer hour, Unsigned stream position]`.
    pub const INDEX: u64 = 1;
}

/// What each state table's refusal names it.
const LABELS: [&str; 4] = [
    "market inventory",
    "market contract terms",
    "lend-lease agreements",
    "purchase contracts",
];

const UNIDENTIFIED: &str = "a market country has no identity";
const PARTY: &str = "a market occurrence's party has no identity";

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved market emit is not valid")
}

/// The rows showing a key's value, by row key: the last row in table order shows it, as the
/// observer's later read overrides an earlier one.
#[derive(Clone, Debug)]
enum Shown {
    One(u128, i128),
    Many(Vec<(u128, i128)>),
}

impl Shown {
    fn set(&mut self, row: u128, value: i128) {
        match self {
            Self::One(held, held_value) if *held == row => *held_value = value,
            Self::One(held, held_value) => {
                let mut rows = vec![(*held, *held_value), (row, value)];
                rows.sort_unstable_by_key(|&(row, _)| row);
                *self = Self::Many(rows);
            }
            Self::Many(rows) => match rows.binary_search_by_key(&row, |&(row, _)| row) {
                Ok(at) => rows[at].1 = value,
                Err(at) => rows.insert(at, (row, value)),
            },
        }
    }

    /// Forgets `row`'s; whether any row still shows the key.
    fn remove(&mut self, row: u128) -> bool {
        match self {
            Self::One(held, _) => *held != row,
            Self::Many(rows) => {
                if let Ok(at) = rows.binary_search_by_key(&row, |&(row, _)| row) {
                    rows.remove(at);
                }
                match rows[..] {
                    [] => false,
                    [(row, value)] => {
                        *self = Self::One(row, value);
                        true
                    }
                    _ => true,
                }
            }
        }
    }

    fn value(&self) -> Option<i128> {
        match self {
            Self::One(_, value) => Some(*value),
            Self::Many(rows) => rows.last().map(|&(_, value)| value),
        }
    }
}

/// A row as the emit holds it: the keys it shows, with their values.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Held {
    values: Vec<(KeyHandle, i128)>,
    orphan: bool,
    unresolved: u32,
}

/// A state table as the emit holds it.
#[derive(Default)]
struct State {
    rows: BTreeMap<u128, Held>,
    orphans: u32,
    unresolved: u64,
    /// Why it is not complete; none while it is, or while it is not observed.
    refusal: Option<Refusal>,
    /// Whether it was read complete.
    complete: bool,
}

/// A chain's length and its open chunk's first hour.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
struct Chain {
    count: u64,
    first: i32,
}

/// Which chain an entry goes to.
#[derive(Clone, Copy)]
enum Chained {
    Stream(u8),
    Party(u32, u8),
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    keys: FastMap<Field, KeyHandle>,
    tables: [State; 4],
    values: FastMap<KeyHandle, Shown>,
    /// Whether the countries' identities and the occurrences are read, and why not.
    identities: Option<Result<(), Refusal>>,
    events: Option<Result<(), Refusal>>,
    /// Each holder slot's tags since its first, with its ordinal, and those changed since the
    /// last cut.
    holders: BTreeMap<u32, (u64, Vec<(i32, String)>)>,
    stale_holders: BTreeSet<u32>,
    /// The occurrence ids of the last read, so one carried twice is kept once.
    last_events: HashSet<u64>,
    streams: BTreeMap<u8, Chain>,
    indexes: BTreeMap<(u32, u8), Chain>,
    /// Occurrences kept so far, across kinds: the next one's sequence.
    sequence: u64,
    lag: i32,
    identified: Vec<(i32, i32)>,
    /// Each refusal's calls across observed hours.
    refusals: BTreeMap<String, u64>,
    /// Whether the Identified flag is off for an occurrence of an hour before, until the next
    /// hour the row preparation observed.
    unproved: bool,
    /// Every variant's naming, the variants the market named with their ordinals, and those
    /// whose naming changed since the last cut.
    catalogue: VariantCatalogue,
    named: BTreeMap<u64, u64>,
    stale_variants: BTreeSet<u64>,
    last: Option<i32>,
    /// The first hour walked since the last cut or the last hour not walked.
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
            keys: FastMap::default(),
            tables: Default::default(),
            values: FastMap::default(),
            identities: None,
            events: None,
            holders: BTreeMap::new(),
            stale_holders: BTreeSet::new(),
            last_events: HashSet::new(),
            streams: BTreeMap::new(),
            indexes: BTreeMap::new(),
            sequence: 0,
            lag: 0,
            identified: Vec::new(),
            refusals: BTreeMap::new(),
            unproved: false,
            catalogue: VariantCatalogue::default(),
            named: BTreeMap::new(),
            stale_variants: BTreeSet::new(),
            last: None,
            first: None,
        }
    }

    /// The key of `field`, interned at `hour` when new.
    fn key(&mut self, writer: &mut Writer, field: Field, hour: i32) -> io::Result<KeyHandle> {
        if let Some(&key) = self.keys.get(&field) {
            return Ok(key);
        }
        let name = serde_json::to_vec(&field)?;
        let actor = self.actors.actor(writer, &name, hour)?;
        let place = Place {
            actor,
            measure: VALUE,
            dimension: None,
        };
        let key = self
            .actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))?;
        engine_allocation::reserve_map(&mut self.keys, 1)?;
        self.keys.insert(field, key);
        Ok(key)
    }

    fn refuse(&mut self, reason: &str, times: u64) {
        if times == 0 {
            return;
        }
        let reason = super::super::keyed::undated(reason);
        match self.refusals.get_mut(reason.as_ref()) {
            Some(count) => *count += times,
            None => {
                self.refusals.insert(reason.into_owned(), times);
            }
        }
    }

    /// Counts each refusal once per call the row preparation makes at an observed hour.
    fn count_refusals(&mut self) {
        let refusals = &mut self.refusals;
        let mut add = |reason: String, times: u64| {
            if times != 0 {
                *refusals
                    .entry(super::super::keyed::undated(&reason).into_owned())
                    .or_default() += times;
            }
        };
        match &self.identities {
            Some(Err(refusal)) => add(format!("country identities unavailable: {refusal}"), 1),
            Some(Ok(())) => {
                for (state, label) in self.tables.iter().zip(LABELS) {
                    match &state.refusal {
                        Some(refusal) => add(format!("{label} unavailable: {refusal}"), 1),
                        None if state.complete => add(UNIDENTIFIED.to_owned(), state.unresolved),
                        None => {}
                    }
                }
            }
            None => {}
        }
        if let Some(Err(refusal)) = &self.events {
            add(format!("market occurrences unavailable: {refusal}"), 1);
        }
    }

    fn name(&mut self, variant: u64) {
        if variant != 0 && !self.named.contains_key(&variant) {
            let ordinal = self.named.len() as u64;
            self.named.insert(variant, ordinal);
            self.stale_variants.insert(variant);
        }
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("market.emit", None);
        let at = hour.hour;
        let adjacent = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => true,
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "A market hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every value; the next observes each again.
            Some(last) => {
                self.close(writer, last)?;
                false
            }
            None => false,
        };
        self.last = Some(at);
        self.first.get_or_insert(at);
        let raws = hour.raws;
        for (id, _) in self
            .catalogue
            .observe_hour_changed(at, hour.variants, |_| raws)
        {
            if self.named.contains_key(&id) {
                self.stale_variants.insert(id);
            }
        }
        // Every walked hour proves its identities or not.
        if hour.identities {
            match self.identified.last_mut() {
                Some(last) if last.1.checked_add(1) == Some(at) => last.1 = at,
                Some(last) if last.1 >= at => {}
                _ => self.identified.push((at, at)),
            }
        }
        match hour.facts {
            Some(facts) => self.observe(writer, at, !adjacent, hour.woke, *facts),
            None if !adjacent => self.observe(writer, at, true, true, facts::Facts::default()),
            None if self.unproved && hour.woke => {
                // The occurrences of an hour before were all it lacked.
                self.unproved = false;
                let shown = matches!(self.events, Some(Ok(())));
                if shown {
                    self.key(writer, Field::Identified, at)?;
                }
                self.flag(writer, Field::Identified, at, shown)
            }
            None => Ok(()),
        }
    }

    /// An observed hour, every value set anew where it follows an hour not walked (`anew`); one
    /// the row preparation observed too where `woke`.
    fn observe(
        &mut self,
        writer: &mut Writer,
        at: i32,
        anew: bool,
        woke: bool,
        facts: facts::Facts,
    ) -> io::Result<()> {
        let facts::Facts {
            identities,
            tags,
            tables,
            events,
        } = facts;
        if let Some(identities) = identities {
            self.identities = Some(identities);
        }
        let identified = matches!(self.identities, Some(Ok(())));
        if let Some(tags) = tags
            && identified
        {
            self.tag_all(&tags, at);
        }
        let mut dirty = Vec::new();
        for (index, table) in tables.into_iter().enumerate() {
            match table {
                Some(Table::Skipped) => {
                    self.clear(index, &mut dirty);
                    self.tables[index].refusal = None;
                    self.tables[index].complete = false;
                }
                Some(Table::Refused(refusal)) => {
                    self.clear(index, &mut dirty);
                    self.tables[index].refusal = Some(refusal);
                    self.tables[index].complete = false;
                }
                Some(Table::Rows { whole, rows }) => {
                    self.tables[index].refusal = None;
                    self.tables[index].complete = true;
                    self.key(writer, Field::Known(index as u8), at)?;
                    if whole {
                        self.clear(index, &mut dirty);
                    }
                    self.rows(writer, at, index, rows, &mut dirty)?;
                }
                None => {}
            }
        }
        let proved = self.events(writer, at, events)?;
        if anew {
            dirty.extend(self.values.keys().copied());
        }
        dirty.sort_unstable();
        dirty.dedup();
        for key in dirty {
            let value = self.values.get(&key).and_then(Shown::value);
            self.carry.value(writer, key, at, value, false)?;
        }
        for (index, state) in self.tables.iter().enumerate() {
            let shown = identified && state.complete && (index == 3 || state.orphans == 0);
            let field = Field::Known(index as u8);
            if let Some(&key) = self.keys.get(&field) {
                self.carry
                    .value(writer, key, at, shown.then_some(1), false)?;
            }
        }
        let read = matches!(self.events, Some(Ok(())));
        self.flag(writer, Field::Known(MARKET_EVENTS), at, read)?;
        // A segment's restatement alone is no hour the row preparation observed.
        if anew || woke || !self.unproved {
            let shown = read && proved;
            if shown {
                self.key(writer, Field::Identified, at)?;
            }
            self.unproved = read && !proved;
            self.flag(writer, Field::Identified, at, shown)?;
        }
        if anew || woke {
            self.count_refusals();
        }
        Ok(())
    }

    /// A flag key's value from `at`: 1 while `shown`, none else.
    fn flag(&mut self, writer: &mut Writer, field: Field, at: i32, shown: bool) -> io::Result<()> {
        if let Some(&key) = self.keys.get(&field) {
            self.carry
                .value(writer, key, at, shown.then_some(1), false)?;
        }
        Ok(())
    }

    /// Records every identified holder's tag at `at` where it changed.
    fn tag_all(&mut self, tags: &[(u32, Box<str>)], at: i32) {
        for (slot, tag) in tags {
            let ordinal = self.holders.len() as u64;
            let (_, runs) = self.holders.entry(*slot).or_insert((ordinal, Vec::new()));
            if runs.last().is_none_or(|(_, last)| **last != **tag) {
                runs.push((at, tag.to_string()));
                self.stale_holders.insert(*slot);
            }
        }
    }

    /// Forgets `row`'s values, its keys `dirty`.
    fn forget(values: &mut FastMap<KeyHandle, Shown>, row: u128, held: Held, dirty: &mut Vec<KeyHandle>) {
        for (key, _) in held.values {
            if let Some(shown) = values.get_mut(&key)
                && !shown.remove(row)
            {
                values.remove(&key);
            }
            dirty.push(key);
        }
    }

    /// Forgets every row of table `index`, its keys `dirty`.
    fn clear(&mut self, index: usize, dirty: &mut Vec<KeyHandle>) {
        let state = &mut self.tables[index];
        for (row, held) in std::mem::take(&mut state.rows) {
            Self::forget(&mut self.values, row, held, dirty);
        }
        state.orphans = 0;
        state.unresolved = 0;
    }

    /// The hour's rows of table `index`: each replaces what it showed before.
    fn rows(
        &mut self,
        writer: &mut Writer,
        at: i32,
        index: usize,
        rows: Vec<(u128, Option<Row>)>,
        dirty: &mut Vec<KeyHandle>,
    ) -> io::Result<()> {
        for (row, read) in rows {
            if let Some(held) = self.tables[index].rows.remove(&row) {
                let state = &mut self.tables[index];
                state.orphans -= u32::from(held.orphan);
                state.unresolved -= u64::from(held.unresolved);
                Self::forget(&mut self.values, row, held, dirty);
            }
            let Some(read) = read else {
                continue;
            };
            for variant in read.variants {
                self.name(variant);
            }
            let mut held = Held {
                values: Vec::with_capacity(read.values.len()),
                orphan: read.orphan,
                unresolved: read.unresolved,
            };
            for (field, value) in read.values {
                let key = self.key(writer, field, at)?;
                match held.values.iter_mut().find(|(held, _)| *held == key) {
                    Some(shown) => shown.1 = value,
                    None => held.values.push((key, value)),
                }
                match self.values.get_mut(&key) {
                    Some(shown) => shown.set(row, value),
                    None => {
                        engine_allocation::reserve_map(&mut self.values, 1)?;
                        self.values.insert(key, Shown::One(row, value));
                    }
                }
                dirty.push(key);
            }
            let state = &mut self.tables[index];
            state.orphans += u32::from(held.orphan);
            state.unresolved += u64::from(held.unresolved);
            state.rows.insert(row, held);
        }
        Ok(())
    }

    /// The hour's occurrences: each new one kept once in its stream and its parties' indexes.
    /// Whether every new one's parties had identities at its own hour.
    fn events(
        &mut self,
        writer: &mut Writer,
        at: i32,
        events: Option<Events>,
    ) -> io::Result<bool> {
        let (ids, kept) = match events {
            Some(Events::Refused(refusal)) => {
                self.events = Some(Err(refusal));
                self.last_events.clear();
                return Ok(true);
            }
            Some(Events::Rows { ids, kept }) => {
                self.events = Some(Ok(()));
                (ids, kept)
            }
            None => return Ok(true),
        };
        self.key(writer, Field::Known(MARKET_EVENTS), at)?;
        let mut proved = true;
        for occurrence in kept {
            if self.last_events.contains(&occurrence.row.id) {
                continue;
            }
            proved &= self.occurrence(writer, at, occurrence)?;
        }
        self.last_events = ids.into_iter().collect();
        Ok(proved)
    }

    /// One new occurrence; whether its parties had identities at its own hour.
    fn occurrence(
        &mut self,
        writer: &mut Writer,
        at: i32,
        Occurrence { row, seller, buyer }: Occurrence,
    ) -> io::Result<bool> {
        self.refuse(PARTY, u64::from(seller.is_none()) + u64::from(buyer.is_none()));
        let spans = &self.identified;
        let proved = spans
            .get(spans.partition_point(|span| span.0 <= row.hour).wrapping_sub(1))
            .is_some_and(|span| row.hour <= span.1);
        let identified = seller.is_some() && buyer.is_some() && proved;
        self.name(row.variant);
        let supplied = row.hour.max(at);
        let (kind, sequence) = (row.kind, self.sequence);
        let party = |slot: Option<u32>| slot.map_or(Cell::Null, |slot| Cell::Unsigned(u64::from(slot)));
        let fields = writer.cells([
            Cell::Unsigned(sequence),
            Cell::Integer(i128::from(row.hour)),
            Cell::Unsigned(row.id),
            Cell::Unsigned(row.occurrence),
            Cell::Unsigned(u64::from(row.kind)),
            Cell::Unsigned(u64::from(row.mode)),
            Cell::Unsigned(row.agreement),
            party(seller),
            party(buyer),
            Cell::Unsigned(row.variant),
            Cell::Integer(i128::from(row.amount_raw)),
            Cell::Integer(i128::from(row.total_raw)),
            Cell::Unsigned(u64::from(row.ordinal)),
        ])?;
        let Some(position) =
            self.append(writer, Chained::Stream(kind), supplied, kind::OCCURRENCE, fields)?
        else {
            self.refuse("a market occurrence could not be kept: a market occurrence chain outgrew its keys", 1);
            return Ok(identified);
        };
        self.sequence += 1;
        let lag = i32::try_from(i64::from(supplied) - i64::from(row.hour)).unwrap_or(i32::MAX);
        self.lag = self.lag.max(lag);
        let parties = [seller, buyer.filter(|buyer| Some(*buyer) != seller)];
        for holder in parties.into_iter().flatten() {
            let fields = writer.cells([
                Cell::Unsigned(sequence),
                Cell::Integer(i128::from(row.hour)),
                Cell::Unsigned(position),
            ])?;
            let chained = Chained::Party(holder, kind);
            if self
                .append(writer, chained, supplied, kind::INDEX, fields)?
                .is_none()
            {
                self.refuse("a market occurrence's party index could not be kept: a market occurrence chain outgrew its keys", 1);
            }
        }
        Ok(identified)
    }

    /// Appends an entry to its chain at `at`, opening its next key every [`CHUNK`] entries;
    /// the entry's position in the chain, none where the chain outgrew its keys.
    fn append(
        &mut self,
        writer: &mut Writer,
        chained: Chained,
        at: i32,
        kind: u64,
        fields: Blob,
    ) -> io::Result<Option<u64>> {
        let chain = match chained {
            Chained::Stream(of) => self.streams.get(&of),
            Chained::Party(holder, of) => self.indexes.get(&(holder, of)),
        }
        .copied()
        .unwrap_or_default();
        let first = if chain.count.is_multiple_of(CHUNK) { at } else { chain.first };
        let Ok(chunk) = u32::try_from(chain.count / CHUNK) else {
            return Ok(None);
        };
        let field = match chained {
            Chained::Stream(of) => Field::Stream(of, chunk, first),
            Chained::Party(holder, of) => Field::Party(holder, of, chunk, first),
        };
        let key = self.key(writer, field, at)?;
        let sequence = writer.sequence()?;
        writer.event(Event {
            key,
            native_hour: at,
            read_hour: Some(at),
            sequence,
            kind,
            fields,
        })?;
        let next = Chain {
            count: chain.count + 1,
            first,
        };
        match chained {
            Chained::Stream(of) => self.streams.insert(of, next),
            Chained::Party(holder, of) => self.indexes.insert((holder, of), next),
        };
        Ok(Some(chain.count))
    }

    /// The walked hours since `first`, through `last`, as known.
    fn domain(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
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

    /// Ends every value after `last`, the hour before one not walked.
    fn close(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.unproved = false;
        self.carry.close(writer, last)?;
        self.domain(writer, last)
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the scalars that changed.
    pub fn cut(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        self.domain(writer, last)?;
        let refusals = Scalar::List(
            self.refusals
                .iter()
                .map(|(reason, hours)| {
                    Scalar::List(vec![
                        Scalar::Text(reason.as_bytes().into()),
                        Scalar::Unsigned(*hours),
                    ])
                })
                .collect(),
        );
        let identified = Scalar::List(
            self.identified
                .iter()
                .map(|&(first, last)| {
                    Scalar::List(vec![
                        Scalar::Integer(i128::from(first)),
                        Scalar::Integer(i128::from(last)),
                    ])
                })
                .collect(),
        );
        for (id, scalar) in [
            (REFUSALS, refusals),
            (LAG, Scalar::Integer(i128::from(self.lag))),
            (IDENTIFIED, identified),
        ] {
            let cell = family::cell(writer, &scalar)?;
            writer.catalogue(id, cell)?;
        }
        for slot in std::mem::take(&mut self.stale_holders) {
            let Some((ordinal, runs)) = self.holders.get(&slot) else {
                continue;
            };
            let scalar = Scalar::List(vec![
                Scalar::Unsigned(u64::from(slot)),
                Scalar::List(
                    runs.iter()
                        .map(|(hour, tag)| {
                            Scalar::List(vec![
                                Scalar::Integer(i128::from(*hour)),
                                Scalar::Text(tag.as_bytes().into()),
                            ])
                        })
                        .collect(),
                ),
            ]);
            let cell = family::cell(writer, &scalar)?;
            writer.catalogue(TAGS + ordinal, cell)?;
        }
        // Every named variant's ordinal is written, its versions none until a read names it: the
        // reader stops at the first ordinal missing.
        for variant in std::mem::take(&mut self.stale_variants) {
            let Some(&ordinal) = self.named.get(&variant) else {
                continue;
            };
            let versions = self
                .catalogue
                .versions
                .get(&variant)
                .map_or(&[][..], Vec::as_slice);
            let scalar = Scalar::List(vec![
                Scalar::Unsigned(variant),
                Scalar::Text(serde_json::to_vec(versions)?.into_boxed_slice()),
            ]);
            let cell = family::cell(writer, &scalar)?;
            writer.catalogue(VARIANTS + ordinal, cell)?;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection reads every row anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let status = |status: &Option<Result<(), Refusal>>| {
            status
                .as_ref()
                .map(|status| status.as_ref().map(|_| ()).map_err(|refusal| refusal.to_string()))
        };
        let mut last_events: Vec<u64> = self.last_events.iter().copied().collect();
        last_events.sort_unstable();
        let mut named: Vec<(u64, u64)> = self.named.iter().map(|(&id, &at)| (at, id)).collect();
        named.sort_unstable();
        let saved = Saved {
            tables: self
                .tables
                .iter()
                .map(|state| SavedTable {
                    rows: state
                        .rows
                        .iter()
                        .map(|(&row, held)| (row, held.clone()))
                        .collect(),
                    refusal: state.refusal.as_ref().map(|refusal| refusal.to_string()),
                    complete: state.complete,
                })
                .collect(),
            identities: status(&self.identities),
            events: status(&self.events),
            holders: self
                .holders
                .iter()
                .map(|(&slot, (ordinal, runs))| (slot, *ordinal, runs.clone()))
                .collect(),
            last_events,
            streams: self.streams.iter().map(|(&kind, chain)| (kind, *chain)).collect(),
            indexes: self
                .indexes
                .iter()
                .map(|(&(holder, kind), chain)| (holder, kind, *chain))
                .collect(),
            sequence: self.sequence,
            lag: self.lag,
            identified: self.identified.clone(),
            refusals: self
                .refusals
                .iter()
                .map(|(reason, hours)| (reason.clone(), *hours))
                .collect(),
            unproved: self.unproved,
            catalogue: Cow::Borrowed(&self.catalogue),
            named: named.into_iter().map(|(_, id)| id).collect(),
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
        for (actor, named) in directory.actors.iter().enumerate() {
            let field: Field = serde_json::from_slice(&named.name).map_err(|_| invalid())?;
            let key = directory
                .key(actor as u64, VALUE, None)
                .ok_or_else(invalid)?;
            emit.keys.insert(field, key);
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved<'static> = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        if saved.tables.len() != emit.tables.len() {
            return Err(invalid());
        }
        for (index, table) in saved.tables.into_iter().enumerate() {
            let state = &mut emit.tables[index];
            state.refusal = table.refusal.map(String::into_boxed_str);
            state.complete = table.complete;
            for (row, held) in table.rows {
                state.orphans += u32::from(held.orphan);
                state.unresolved += u64::from(held.unresolved);
                for &(key, value) in &held.values {
                    match emit.values.get_mut(&key) {
                        Some(shown) => shown.set(row, value),
                        None => {
                            emit.values.insert(key, Shown::One(row, value));
                        }
                    }
                }
                state.rows.insert(row, held);
            }
        }
        let status = |status: Option<Result<(), String>>| {
            status.map(|status| status.map_err(String::into_boxed_str))
        };
        emit.identities = status(saved.identities);
        emit.events = status(saved.events);
        emit.holders = saved
            .holders
            .into_iter()
            .map(|(slot, ordinal, runs)| (slot, (ordinal, runs)))
            .collect();
        emit.last_events = saved.last_events.into_iter().collect();
        emit.streams = saved.streams.into_iter().collect();
        emit.indexes = saved
            .indexes
            .into_iter()
            .map(|(holder, kind, chain)| ((holder, kind), chain))
            .collect();
        emit.sequence = saved.sequence;
        emit.lag = saved.lag;
        emit.identified = saved.identified;
        emit.refusals = saved.refusals.into_iter().collect();
        emit.unproved = saved.unproved;
        emit.catalogue = saved.catalogue.into_owned();
        emit.named = saved
            .named
            .into_iter()
            .enumerate()
            .map(|(ordinal, id)| (id, ordinal as u64))
            .collect();
        emit.last = saved.last;
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct SavedTable {
    rows: Vec<(u128, Held)>,
    refusal: Option<String>,
    complete: bool,
}

#[derive(Serialize, Deserialize)]
struct Saved<'a> {
    tables: Vec<SavedTable>,
    identities: Option<Result<(), String>>,
    events: Option<Result<(), String>>,
    holders: Vec<(u32, u64, Vec<(i32, String)>)>,
    last_events: Vec<u64>,
    streams: Vec<(u8, Chain)>,
    indexes: Vec<(u32, u8, Chain)>,
    sequence: u64,
    lag: i32,
    identified: Vec<(i32, i32)>,
    refusals: Vec<(String, u64)>,
    unproved: bool,
    catalogue: Cow<'a, VariantCatalogue>,
    named: Vec<u64>,
    last: Option<i32>,
}

/// A variant's naming reads, as [`Emit::cut`] spells them.
pub fn versions(text: &[u8]) -> io::Result<Vec<VariantVersion>> {
    Ok(serde_json::from_slice(text)?)
}
