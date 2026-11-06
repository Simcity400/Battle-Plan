//! Diplomacy's emit: each country's identity, opinion read and relation read, each owner's
//! opinion of each target, each input's coverage and the unresolved count, as runs on the
//! family's series publication; and each pact and proposal as a catalogued record, written at a
//! cut when it changed, with the relation kinds' spellings and the reads present pacts were
//! last seen at.
//!
//! A country's identity run starts where it joins and where the hours stop going on: a gap or
//! a refused hour. An owner's read proves its pairs only toward identities that existed at it.
//! Pacts go on while their raw owner's reads list them, proposals while the queue's fresh reads
//! do; an unread, refused or unavailable hour ends them as bounded.

use super::{
    DiplomacyCountry,
    facts::{self, Ids, Opinion, Opinions, Queue, QueueRead, TupleOwner, TupleRows, Tuples},
};
use dashboard_series::{
    Cell, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
    hash::FastMap,
};
use replay_data::cadence::STAGGER_PERIOD;
use serde::{Deserialize, Serialize};
use std::{io, sync::Arc};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A country's measures: 1 while it is live; its opinion read, twice the supplying hour plus
/// whether the read is known; its relation read's supplying hour; its opinion of each target,
/// split by the target's actor.
pub const IDENTITY: u64 = 0;
pub const OWNER: u64 = 1;
pub const TUPLES: u64 = 2;
pub const OPINION: u64 = 3;
/// The family's measures: 1 while opinions, and relations, are read; the queue's last fresh
/// read; the handles a fresh read repeats; the identities and rows no country resolves.
pub const OPINION_COVERAGE: u64 = 4;
pub const TUPLE_COVERAGE: u64 = 5;
pub const QUEUE_COVERAGE: u64 = 6;
pub const QUEUE_DUPLICATES: u64 = 7;
pub const UNRESOLVED: u64 = 8;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// Family scalars: each raw relation owner holding a present pact and its last read, as
/// flattened pairs; each kind's text, latest token and whether it is unresolved; each pact
/// ([`pact_cell`]); each proposal ([`proposal_cell`]); the read each group of proposals left
/// awaiting their handle's absence was gone by; by actor, the opinion read a departed identity
/// takes up again should it come back.
pub const RAW_READS: u64 = 0;
pub const KINDS: u64 = 1 << 20;
pub const PACTS: u64 = 1 << 24;
pub const PROPOSALS: u64 = 1 << 29;
pub const GONE: u64 = 1 << 30;
pub const RETAINED: u64 = 1 << 31;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;
/// No country: a role no identity resolves.
const NOBODY: u64 = u64::MAX;

/// A country's actor name: its slot, id and tag.
pub fn country_name(country: &DiplomacyCountry) -> Vec<u8> {
    let mut name = Vec::with_capacity(8 + country.tag.len());
    name.extend_from_slice(&country.slot.to_le_bytes());
    name.extend_from_slice(&country.country_id.to_le_bytes());
    name.extend_from_slice(country.tag.as_bytes());
    name
}

/// The country a [`country_name`] names.
pub fn country_of(name: &[u8]) -> Option<DiplomacyCountry> {
    if name.len() < 8 {
        return None;
    }
    Some(DiplomacyCountry {
        slot: u32::from_le_bytes(name[..4].try_into().ok()?),
        country_id: u32::from_le_bytes(name[4..8].try_into().ok()?),
        tag: std::str::from_utf8(&name[8..]).ok()?.to_owned(),
    })
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved diplomacy emit is not valid")
}

fn scalar_id(base: u64, at: u64, limit: u64) -> io::Result<u64> {
    base.checked_add(at)
        .filter(|&id| id < limit)
        .ok_or_else(|| io::Error::other("Diplomacy catalogue ids are exhausted"))
}

fn deadline(read: i32) -> i64 {
    i64::from(read) + i64::from(STAGGER_PERIOD)
}

/// Held reads by the hour they expire: a read expires within a cadence period of its hour.
struct Deadlines<K> {
    buckets: Vec<Vec<(i64, K)>>,
}

impl<K: Copy> Default for Deadlines<K> {
    fn default() -> Self {
        Self {
            buckets: (0..STAGGER_PERIOD).map(|_| Vec::new()).collect(),
        }
    }
}

impl<K: Copy> Deadlines<K> {
    fn insert(&mut self, read: i32, key: K) {
        let at = deadline(read);
        self.buckets[at.rem_euclid(i64::from(STAGGER_PERIOD)) as usize].push((at, key));
    }

    /// The reads expiring at `hour`; earlier ones are gone with what held them.
    fn due(&mut self, hour: i32, out: &mut Vec<K>) {
        out.clear();
        let at = i64::from(hour);
        let bucket = &mut self.buckets[at.rem_euclid(i64::from(STAGGER_PERIOD)) as usize];
        bucket.retain(|&(expires, key)| {
            if expires == at {
                out.push(key);
            }
            expires > at
        });
    }

    fn clear(&mut self) {
        for bucket in &mut self.buckets {
            bucket.clear();
        }
    }
}

/// A relation kind: its token's spelling, or the token no spelling supplies.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Kind {
    Spelled(Arc<str>),
    Unresolved(u32),
}

impl Kind {
    fn text(&self) -> String {
        match self {
            Self::Spelled(spelling) => spelling.to_string(),
            Self::Unresolved(token) => format!("unresolved-token-{token}"),
        }
    }
}

/// What makes a relation row the same pact: its key, endpoints, start, the roles its read
/// resolved and its kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct Signature {
    raw: [u32; 4],
    type_id: u32,
    ordinal: u16,
    start: i32,
    roles: [u64; 4],
    kind: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pact {
    signature: Signature,
    /// The hour it was first seen.
    created: i32,
    first_seen: i32,
    end: i32,
    cancel: u8,
    retired: u8,
    /// Once it ended: its last read and whether its end is only bounded.
    ended: Option<(i32, bool)>,
    #[serde(skip)]
    dirty: bool,
}

/// What makes a queued action the same proposal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
struct QueueSignature {
    handle: u64,
    raw: [u32; 5],
    roles: [u64; 5],
    stored: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Proposal {
    signature: QueueSignature,
    created: i32,
    last_command: i32,
    first_seen: i32,
    last_seen: i32,
    gone_by: Option<i32>,
    bound: bool,
    present: bool,
    /// Left awaiting a read without its handle, it is gone by the read its group is.
    group: Option<u64>,
    /// The fresh read that last listed it.
    #[serde(skip)]
    found: Option<i32>,
    #[serde(skip)]
    dirty: bool,
}

#[derive(Clone, Copy, Default)]
struct Country {
    /// Identity, owner and relation read.
    keys: [KeyHandle; 3],
    /// The first hour of its open identity run; none while it is not live.
    identity_first: Option<i32>,
    /// Its index in the identities the hours go on with.
    index: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
struct Pair {
    target: u64,
    key: KeyHandle,
    value: i32,
}

#[derive(Default)]
struct Owner {
    read: Option<i32>,
    known: bool,
    unresolved: usize,
    /// The values the pairs were stored from, while they still are: the pairs are in their
    /// targets' order.
    values: Option<Arc<facts::Values>>,
    pairs: Vec<Pair>,
}

struct RawRead {
    read: i32,
    memo: Option<Memo>,
}

struct Memo {
    rows: Arc<TupleRows>,
    pacts: Vec<u64>,
    /// Each kind the rows name and the token of the last row naming it.
    kinds: Vec<(u32, u32)>,
    read: i32,
    /// Every endpoint a country resolves was resolved at the read.
    settled: bool,
}

/// The family's emit on one branch.
#[derive(Default)]
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// Opinion, relation and queue coverage, duplicates and unresolved.
    family: Option<[KeyHandle; 5]>,
    /// By actor id.
    countries: Vec<Country>,
    /// The identities the hours go on with, and each country's actor by index.
    ids: Option<Arc<Ids>>,
    named: Vec<u64>,
    /// The live countries, ascending.
    live: Vec<u64>,
    owners: FastMap<u64, Owner>,
    owners_unresolved: usize,
    orphan_opinions: usize,
    tuple_reads: FastMap<u64, i32>,
    raw_reads: FastMap<u32, RawRead>,
    opinion_deadlines: Deadlines<u64>,
    tuple_deadlines: Deadlines<u64>,
    raw_deadlines: Deadlines<u32>,
    /// Present pacts, and those ended since the last cut.
    pacts: FastMap<u64, Pact>,
    present: FastMap<Signature, u64>,
    /// Each raw owner's present pacts, ascending.
    by_owner: FastMap<u32, Vec<u64>>,
    next_pact: u64,
    /// Present proposals, and those changed since the last cut.
    proposals: FastMap<u64, Proposal>,
    queued: FastMap<QueueSignature, u64>,
    numbered: FastMap<u128, (Arc<facts::Proposal>, u64)>,
    queue_read: Option<i32>,
    next_proposal: u64,
    /// The open group of each owner's handle whose proposals await a read without it.
    awaiting: FastMap<(u32, u64), u64>,
    next_group: u64,
    /// Groups gone by a read since the last cut.
    gone: Vec<(u64, i32)>,
    /// Each departed identity's opinion read, by actor.
    retained: FastMap<u64, i128>,
    /// The actors whose retained read changed since the last cut.
    retained_changed: Vec<u64>,
    kinds: FastMap<Kind, u32>,
    /// Each kind and its latest token, by id.
    kind_list: Vec<(Kind, u32)>,
    dirty_kinds: Vec<u32>,
    dirty_pacts: Vec<u64>,
    dirty_proposals: Vec<u64>,
    /// The reads [`RAW_READS`] holds.
    written_reads: Vec<(u32, i32)>,
    /// Whether the hours go on: none before the first, or after a refused one.
    segment_first: Option<i32>,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
    /// Scratch for expiring reads.
    due: Vec<u64>,
    due_raw: Vec<u32>,
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            ..Self::default()
        }
    }

    fn key(
        &mut self,
        writer: &mut Writer,
        actor: u64,
        measure: u64,
        dimension: Option<u64>,
    ) -> io::Result<KeyHandle> {
        self.actors.key(
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

    fn family_keys(&mut self, writer: &mut Writer) -> io::Result<[KeyHandle; 5]> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let mut keys = [0; 5];
        for (at, key) in keys.iter_mut().enumerate() {
            *key = self.key(writer, FAMILY, OPINION_COVERAGE + at as u64, None)?;
        }
        self.family = Some(keys);
        Ok(keys)
    }

    fn country(
        &mut self,
        writer: &mut Writer,
        country: &DiplomacyCountry,
        hour: i32,
    ) -> io::Result<u64> {
        let name = country_name(country);
        if let Some(actor) = self.actors.find(&name) {
            return Ok(actor);
        }
        let actor = self.actors.actor(writer, &name, hour)?;
        let mut keys = [0; 3];
        for (measure, key) in keys.iter_mut().enumerate() {
            *key = self.key(writer, actor, measure as u64, None)?;
        }
        if actor as usize >= self.countries.len() {
            self.countries.resize(actor as usize + 1, Country::default());
        }
        self.countries[actor as usize].keys = keys;
        Ok(actor)
    }

    fn value(
        &mut self,
        writer: &mut Writer,
        key: KeyHandle,
        hour: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()> {
        self.carry.value(writer, key, hour, value, starts).map(drop)
    }

    fn number(
        &mut self,
        writer: &mut Writer,
        at: usize,
        hour: i32,
        value: Option<i128>,
    ) -> io::Result<()> {
        let key = self.family_keys(writer)?[at];
        self.value(writer, key, hour, value, false)
    }

    fn identity(
        &mut self,
        writer: &mut Writer,
        actor: u64,
        hour: i32,
        live: bool,
    ) -> io::Result<()> {
        let country = &mut self.countries[actor as usize];
        country.identity_first = live.then_some(hour);
        let key = country.keys[0];
        self.value(writer, key, hour, live.then_some(1), live)
    }

    fn owner_read(
        &mut self,
        writer: &mut Writer,
        actor: u64,
        hour: i32,
        read: Option<i32>,
        known: bool,
    ) -> io::Result<()> {
        let key = self.countries[actor as usize].keys[1];
        let value = read.map(|read| i128::from(read) * 2 + i128::from(known));
        self.value(writer, key, hour, value, false)
    }

    fn tuple_read(
        &mut self,
        writer: &mut Writer,
        actor: u64,
        hour: i32,
        read: Option<i32>,
    ) -> io::Result<()> {
        let key = self.countries[actor as usize].keys[2];
        self.value(writer, key, hour, read.map(i128::from), false)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("emit", Some("diplomacy_state.hour"));
        let at = hour.hour;
        self.family_keys(writer)?;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last.filter(|_| !adjacent) {
            // An unread hour ends every run, and what departed identities kept with it.
            self.carry.close(writer, last)?;
            // Taken, not drained: a drained map keeps its peak's capacity for every later gap.
            self.retained_changed
                .extend(std::mem::take(&mut self.retained).into_keys());
        }
        let continuous = self.segment_first.is_some() && adjacent;
        if !continuous {
            self.end_pacts(None, true)?;
            self.suspend_queue()?;
            self.close_owners(writer, at)?;
            self.close_tuple_reads(writer, at)?;
        }
        self.segment_first.get_or_insert(at);
        self.last = Some(at);
        self.first.get_or_insert(at);
        if hour.refused {
            return self.refuse(writer, at);
        }
        if self.segment_first == Some(at) && !hour.baseline {
            return Err(io::Error::other(
                "A projected hour lacks the baseline its continuation admits",
            ));
        }
        if let Some(ids) = hour.ids {
            self.name(writer, ids, at, continuous)?;
        } else if self.ids.is_none() {
            return Err(io::Error::other("A projected hour lacks its identities"));
        } else if !continuous {
            // Kept identities hold exactly the live countries, so each one goes on.
            for at_live in 0..self.live.len() {
                let actor = self.live[at_live];
                self.identity(writer, actor, at, true)?;
            }
        }
        let ids = Arc::clone(self.ids.as_ref().expect("the hour's identities"));
        self.opinions(writer, at, hour.opinions)?;
        self.tuples(writer, at, &ids, hour.tuples)?;
        self.queue(writer, at, hour.queue)?;
        let unresolved = ids.unresolved + self.orphan_opinions + self.owners_unresolved;
        self.number(writer, 4, at, Some(unresolved as i128))
    }

    /// Takes up the hour's identities: countries leaving end what they held, and those
    /// joining, or every one after a break, start their identity runs.
    fn name(
        &mut self,
        writer: &mut Writer,
        ids: Arc<Ids>,
        at: i32,
        continuous: bool,
    ) -> io::Result<()> {
        // A memo's roles were dated with the identities it was observed with.
        for held in self.raw_reads.values_mut() {
            held.memo = None;
        }
        for &actor in &self.live {
            self.countries[actor as usize].index = None;
        }
        self.named.clear();
        let mut live = Vec::with_capacity(ids.countries.len());
        for (index, country) in ids.countries.iter().enumerate() {
            let actor = self.country(writer, country, at)?;
            self.countries[actor as usize].index = Some(index as u32);
            self.named.push(actor);
            live.push(actor);
        }
        live.sort_unstable();
        let removed: Vec<u64> = self
            .live
            .iter()
            .copied()
            .filter(|actor| live.binary_search(actor).is_err())
            .collect();
        if !removed.is_empty() {
            self.end_pacts(Some(&removed), true)?;
            self.end_queue_identities(&removed)?;
        }
        for &actor in &removed {
            self.close_owner(writer, actor, at)?;
        }
        self.close_targets(writer, &removed, at)?;
        for &actor in &removed {
            self.tuple_reads.remove(&actor);
            self.tuple_read(writer, actor, at, None)?;
            self.identity(writer, actor, at, false)?;
            self.retire(writer, actor, at)?;
        }
        for &actor in &live {
            // A new segment restarts every continuing identity; one that just joined starts
            // its run once.
            let joined = self.live.binary_search(&actor).is_err();
            if !continuous || joined {
                self.identity(writer, actor, at, true)?;
            }
            if joined {
                self.rejoin(writer, actor, at)?;
            }
        }
        self.live = live;
        self.ids = Some(ids);
        Ok(())
    }

    fn refuse(&mut self, writer: &mut Writer, at: i32) -> io::Result<()> {
        self.close_owners(writer, at)?;
        self.end_pacts(None, true)?;
        self.suspend_queue()?;
        self.close_tuple_reads(writer, at)?;
        for actor in std::mem::take(&mut self.live) {
            self.identity(writer, actor, at, false)?;
            self.retire(writer, actor, at)?;
            self.countries[actor as usize].index = None;
        }
        for coverage in 0..3 {
            self.number(writer, coverage, at, None)?;
        }
        self.number(writer, 4, at, Some(1))?;
        self.segment_first = None;
        self.ids = None;
        self.named.clear();
        Ok(())
    }

    /// The actor of a role read at `read`: a country whose identity existed then.
    fn role_at_read(&self, ids: &Ids, raw: u32, read: i32) -> Option<u64> {
        let actor = self.named[ids.index_of(raw)? as usize];
        self.countries[actor as usize]
            .identity_first
            .is_some_and(|first| first <= read)
            .then_some(actor)
    }

    fn close_owner(&mut self, writer: &mut Writer, actor: u64, at: i32) -> io::Result<()> {
        let Some(owner) = self.owners.remove(&actor) else {
            return Ok(());
        };
        self.owners_unresolved -= owner.unresolved;
        for pair in &owner.pairs {
            self.value(writer, pair.key, at, None, false)?;
        }
        // The supplying read stays where the owner's proof ends.
        self.owner_read(writer, actor, at, owner.read, false)
    }

    fn close_owners(&mut self, writer: &mut Writer, at: i32) -> io::Result<()> {
        let mut held: Vec<u64> = self.owners.keys().copied().collect();
        held.sort_unstable();
        for actor in held {
            self.close_owner(writer, actor, at)?;
        }
        self.opinion_deadlines.clear();
        Ok(())
    }

    /// Ends every pair toward one of `targets` (ascending), in one pass over the owners.
    fn close_targets(
        &mut self,
        writer: &mut Writer,
        targets: &[u64],
        at: i32,
    ) -> io::Result<()> {
        if targets.is_empty() {
            return Ok(());
        }
        let mut ended = Vec::new();
        for owner in self.owners.values_mut() {
            let held = owner.pairs.len();
            owner.pairs.retain(|pair| {
                let gone = targets.binary_search(&pair.target).is_ok();
                if gone {
                    ended.push(pair.key);
                }
                !gone
            });
            if owner.pairs.len() != held {
                owner.values = None;
            }
        }
        ended.sort_unstable();
        for key in ended {
            self.value(writer, key, at, None, false)?;
        }
        Ok(())
    }

    /// A departing identity's opinion read stays with it, off the carry, until it comes
    /// back or an unread hour ends it.
    fn retire(&mut self, writer: &mut Writer, actor: u64, at: i32) -> io::Result<()> {
        let key = self.countries[actor as usize].keys[1];
        let Some(value) = self.carry.held(key) else {
            return Ok(());
        };
        scalar_id(RETAINED, actor, family::ACTORS)?;
        self.value(writer, key, at, None, false)?;
        self.retained.insert(actor, value);
        self.retained_changed.push(actor);
        Ok(())
    }

    fn rejoin(&mut self, writer: &mut Writer, actor: u64, at: i32) -> io::Result<()> {
        let Some(value) = self.retained.remove(&actor) else {
            return Ok(());
        };
        self.retained_changed.push(actor);
        let key = self.countries[actor as usize].keys[1];
        self.value(writer, key, at, Some(value), false)
    }

    fn close_tuple_reads(&mut self, writer: &mut Writer, at: i32) -> io::Result<()> {
        self.raw_reads.clear();
        let mut held: Vec<u64> = self.tuple_reads.drain().map(|(actor, _)| actor).collect();
        held.sort_unstable();
        for actor in held {
            self.tuple_read(writer, actor, at, None)?;
        }
        self.tuple_deadlines.clear();
        self.raw_deadlines.clear();
        Ok(())
    }

    fn opinions(
        &mut self,
        writer: &mut Writer,
        at: i32,
        opinions: Option<Opinions>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("emit", Some("diplomacy_state.opinions"));
        self.number(writer, 0, at, opinions.as_ref().map(|_| 1))?;
        let Some(Opinions {
            current,
            baseline,
            orphans,
        }) = opinions
        else {
            self.close_owners(writer, at)?;
            self.orphan_opinions = 0;
            return Ok(());
        };
        let mut due = std::mem::take(&mut self.due);
        self.opinion_deadlines.due(at, &mut due);
        for &actor in &due {
            let selected = self.countries[actor as usize].index.is_some_and(|index| {
                current
                    .binary_search_by_key(&index, |opinion| opinion.0)
                    .is_ok()
            });
            if !selected
                && self
                    .owners
                    .get(&actor)
                    .and_then(|owner| owner.read)
                    .is_some_and(|read| deadline(read) == i64::from(at))
            {
                self.close_owner(writer, actor, at)?;
            }
        }
        self.due = due;
        self.orphan_opinions = orphans;
        let current = if self.segment_first == Some(at) {
            let (owners, named) = (&self.owners, &self.named);
            facts::merged(
                current,
                baseline
                    .into_iter()
                    .filter(|opinion| !owners.contains_key(&named[opinion.0 as usize])),
                |opinion| opinion.0,
            )
        } else {
            current
        };
        for opinion in current {
            self.store_opinion(writer, at, opinion)?;
        }
        Ok(())
    }

    fn store_opinion(
        &mut self,
        writer: &mut Writer,
        at: i32,
        (index, read, values): Opinion,
    ) -> io::Result<()> {
        let actor = self.named[index as usize];
        let known = read.is_some() && values.unresolved == 0;
        if let Some(read) = read {
            self.opinion_deadlines.insert(read, actor);
        }
        let Some(owner) = self.owners.get_mut(&actor) else {
            return self.store_owner(writer, at, actor, read, values);
        };
        let held = owner.values.as_ref();
        if held.is_some_and(|held| owner.known == known && Arc::ptr_eq(held, &values)) {
            owner.read = read;
        } else if held.is_some_and(|held| {
            owner.known && known && Arc::ptr_eq(&held.targets, &values.targets)
        }) {
            // The same targets hold the same pairs, so only a changed value is observed.
            let mut changed = Vec::new();
            for (pair, &value) in owner.pairs.iter_mut().zip(values.values.iter()) {
                if pair.value != value {
                    pair.value = value;
                    changed.push((pair.key, value));
                }
            }
            owner.values = Some(values);
            owner.read = read;
            for (key, value) in changed {
                self.value(writer, key, at, Some(i128::from(value)), false)?;
            }
        } else {
            return self.store_owner(writer, at, actor, read, values);
        }
        self.owner_read(writer, actor, at, read, known)
    }

    /// Stores an owner's read whole, against what it held.
    fn store_owner(
        &mut self,
        writer: &mut Writer,
        at: i32,
        actor: u64,
        read: Option<i32>,
        values: Arc<facts::Values>,
    ) -> io::Result<()> {
        let known = read.is_some() && values.unresolved == 0;
        let mut old = self.owners.remove(&actor).unwrap_or_default();
        self.owners_unresolved = self.owners_unresolved - old.unresolved + values.unresolved;
        old.pairs.sort_unstable_by_key(|pair| pair.target);
        let mut kept = vec![false; old.pairs.len()];
        let mut pairs = Vec::new();
        if known {
            pairs.reserve_exact(values.targets.len());
            for (&target, &value) in values.targets.iter().zip(values.values.iter()) {
                let target = self.named[target as usize];
                let key = match old.pairs.binary_search_by_key(&target, |pair| pair.target) {
                    Ok(found) => {
                        kept[found] = true;
                        old.pairs[found].key
                    }
                    Err(_) => self.key(writer, actor, OPINION, Some(target))?,
                };
                self.value(writer, key, at, Some(i128::from(value)), !old.known)?;
                pairs.push(Pair { target, key, value });
            }
        }
        for (pair, kept) in old.pairs.iter().zip(kept) {
            if !kept {
                self.value(writer, pair.key, at, None, false)?;
            }
        }
        self.owners.insert(
            actor,
            Owner {
                read,
                known,
                unresolved: values.unresolved,
                values: Some(values),
                pairs,
            },
        );
        self.owner_read(writer, actor, at, read, known)
    }

    fn tuples(
        &mut self,
        writer: &mut Writer,
        at: i32,
        ids: &Ids,
        tuples: Option<Tuples>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("emit", Some("diplomacy_state.tuples"));
        self.number(writer, 1, at, tuples.as_ref().map(|_| 1))?;
        let Some(Tuples {
            countries,
            baseline_countries,
            owners,
            baseline_owners,
        }) = tuples
        else {
            self.end_pacts(None, true)?;
            return self.close_tuple_reads(writer, at);
        };
        let mut due = std::mem::take(&mut self.due);
        self.tuple_deadlines.due(at, &mut due);
        for &actor in &due {
            let index = self.countries[actor as usize].index;
            let selected = index.is_some_and(|index| {
                countries
                    .binary_search_by_key(&index, |read| read.0)
                    .is_ok()
            });
            if selected
                || self
                    .tuple_reads
                    .get(&actor)
                    .is_none_or(|&read| deadline(read) != i64::from(at))
            {
                continue;
            }
            if let Some(index) = index {
                for &raw in ids.aliases[index as usize].iter() {
                    self.end_owner_pacts(raw)?;
                }
            }
            self.tuple_reads.remove(&actor);
            self.tuple_read(writer, actor, at, None)?;
        }
        self.due = due;
        let (countries, owners) = if self.segment_first == Some(at) {
            (
                facts::merged(countries, baseline_countries, |read| read.0),
                facts::merged(owners, baseline_owners, |owner| owner.0),
            )
        } else {
            (countries, owners)
        };
        for (index, read) in countries {
            let actor = self.named[index as usize];
            self.tuple_read(writer, actor, at, Some(read))?;
            self.tuple_deadlines.insert(read, actor);
            self.tuple_reads.insert(actor, read);
        }
        let mut due = std::mem::take(&mut self.due_raw);
        self.raw_deadlines.due(at, &mut due);
        for &raw in &due {
            if owners.binary_search_by_key(&raw, |owner| owner.0).is_ok()
                || self
                    .raw_reads
                    .get(&raw)
                    .is_none_or(|held| deadline(held.read) != i64::from(at))
            {
                continue;
            }
            self.end_owner_pacts(raw)?;
            self.raw_reads.remove(&raw);
        }
        self.due_raw = due;
        for owner in owners {
            self.store_tuples(at, ids, owner)?;
        }
        Ok(())
    }

    fn store_tuples(&mut self, at: i32, ids: &Ids, (raw, read, rows): TupleOwner) -> io::Result<()> {
        self.raw_deadlines.insert(read, raw);
        let (previous, memo) = match self.raw_reads.get_mut(&raw) {
            Some(held) => (
                Some(std::mem::replace(&mut held.read, read)),
                held.memo.take(),
            ),
            None => {
                self.raw_reads.insert(raw, RawRead { read, memo: None });
                (None, None)
            }
        };
        // Endpoints resolved at an earlier read stay resolved at a later one.
        let memo = memo.filter(|memo| {
            Arc::ptr_eq(&memo.rows.identity, &rows.identity)
                && memo.read <= read
                && (memo.settled || memo.read == read)
        });
        let memo = match memo {
            Some(mut memo) => {
                if !Arc::ptr_eq(&memo.rows, &rows) {
                    for (&ordinal, row) in memo.pacts.iter().zip(rows.rows.iter()) {
                        self.update_pact(ordinal, row);
                    }
                }
                // Every read of a kind names its latest token, as a whole read does.
                for &(kind, token) in &memo.kinds {
                    self.kind_token(kind, token);
                }
                memo.rows = rows;
                memo.read = read;
                memo
            }
            None => {
                let mut pacts = Vec::with_capacity(rows.rows.len());
                let mut kinds: Vec<(u32, u32)> = Vec::new();
                let mut settled = true;
                for (row, spelling) in rows.rows.iter().zip(rows.identity.iter()) {
                    let (ordinal, kind) = self.observe_pact(at, read, ids, row, spelling)?;
                    pacts.push(ordinal);
                    match kinds.iter_mut().find(|(held, _)| *held == kind) {
                        Some(held) => held.1 = row.type_id,
                        None => kinds.push((kind, row.type_id)),
                    }
                    for raw in [
                        row.owner_country,
                        row.target_country,
                        row.first_country,
                        row.second_country,
                    ] {
                        settled &= ids.index_of(raw).is_none()
                            || self.role_at_read(ids, raw, read).is_some();
                    }
                }
                let mut listed = pacts.clone();
                listed.sort_unstable();
                listed.dedup();
                if let Some(held) = self.by_owner.get_mut(&raw) {
                    // Both ascending: what the read did not list is gone from it.
                    let mut missing = Vec::new();
                    held.retain(|ordinal| {
                        let kept = listed.binary_search(ordinal).is_ok();
                        if !kept {
                            missing.push(*ordinal);
                        }
                        kept
                    });
                    if held.is_empty() {
                        self.by_owner.remove(&raw);
                    }
                    if !missing.is_empty() {
                        // Every present pact was listed by the owner's read before this one.
                        let previous = previous.ok_or_else(|| {
                            io::Error::other("A present pact's owner holds no read")
                        })?;
                        for ordinal in missing {
                            self.end_pact(ordinal, false, previous);
                        }
                    }
                }
                Memo {
                    rows,
                    pacts,
                    kinds,
                    read,
                    settled,
                }
            }
        };
        self.raw_reads.get_mut(&raw).expect("held raw read").memo = Some(memo);
        Ok(())
    }

    fn kind_token(&mut self, id: u32, token: u32) {
        if self.kind_list[id as usize].1 != token {
            self.kind_list[id as usize].1 = token;
            self.dirty_kinds.push(id);
        }
    }

    fn kind(&mut self, spelling: &Option<Arc<str>>, token: u32) -> io::Result<u32> {
        let kind = match spelling {
            Some(spelling) => Kind::Spelled(Arc::clone(spelling)),
            None => Kind::Unresolved(token),
        };
        if let Some(&id) = self.kinds.get(&kind) {
            self.kind_token(id, token);
            return Ok(id);
        }
        let id = self.kind_list.len() as u32;
        scalar_id(KINDS, u64::from(id), PACTS)?;
        self.kinds.insert(kind.clone(), id);
        self.kind_list.push((kind, token));
        self.dirty_kinds.push(id);
        Ok(id)
    }

    fn observe_pact(
        &mut self,
        at: i32,
        read: i32,
        ids: &Ids,
        row: &replay_data::rows::RelationTupleRow,
        spelling: &Option<Arc<str>>,
    ) -> io::Result<(u64, u32)> {
        let raw = [
            row.owner_country,
            row.target_country,
            row.first_country,
            row.second_country,
        ];
        let signature = Signature {
            raw,
            type_id: row.type_id,
            ordinal: row.ordinal,
            start: row.start_hour,
            roles: raw.map(|raw| self.role_at_read(ids, raw, read).unwrap_or(NOBODY)),
            kind: self.kind(spelling, row.type_id)?,
        };
        let ordinal = match self.present.get(&signature) {
            Some(&ordinal) => ordinal,
            None => {
                let ordinal = self.next_pact;
                scalar_id(PACTS, ordinal, PROPOSALS)?;
                self.next_pact += 1;
                self.pacts.insert(
                    ordinal,
                    Pact {
                        signature,
                        created: at,
                        first_seen: read,
                        end: row.end_hour,
                        cancel: row.cancel,
                        retired: row.retired,
                        ended: None,
                        dirty: true,
                    },
                );
                self.dirty_pacts.push(ordinal);
                self.present.insert(signature, ordinal);
                // Ordinals only grow: each owner's stay ascending.
                self.by_owner.entry(raw[0]).or_default().push(ordinal);
                return Ok((ordinal, signature.kind));
            }
        };
        self.update_pact(ordinal, row);
        Ok((ordinal, signature.kind))
    }

    fn update_pact(&mut self, ordinal: u64, row: &replay_data::rows::RelationTupleRow) {
        let pact = self.pacts.get_mut(&ordinal).expect("present pact");
        if (pact.end, pact.cancel, pact.retired) != (row.end_hour, row.cancel, row.retired) {
            pact.end = row.end_hour;
            pact.cancel = row.cancel;
            pact.retired = row.retired;
            if !std::mem::replace(&mut pact.dirty, true) {
                self.dirty_pacts.push(ordinal);
            }
        }
    }

    /// Ends a present pact, last seen at `read`; its caller takes it out of `by_owner`.
    fn end_pact(&mut self, ordinal: u64, bounded: bool, read: i32) {
        let pact = self.pacts.get_mut(&ordinal).expect("present pact");
        pact.ended = Some((read, bounded));
        if !std::mem::replace(&mut pact.dirty, true) {
            self.dirty_pacts.push(ordinal);
        }
        let (signature, owner) = (pact.signature, pact.signature.raw[0]);
        self.present.remove(&signature);
        if let Some(held) = self.raw_reads.get_mut(&owner) {
            held.memo = None;
        }
    }

    fn last_read(&self, owner: u32) -> io::Result<i32> {
        self.raw_reads
            .get(&owner)
            .map(|held| held.read)
            .ok_or_else(|| io::Error::other("A present pact's owner holds no read"))
    }

    /// Ends `raw`'s present pacts as bounded.
    fn end_owner_pacts(&mut self, raw: u32) -> io::Result<()> {
        if !self.by_owner.contains_key(&raw) {
            return Ok(());
        }
        let read = self.last_read(raw)?;
        for ordinal in self.by_owner.remove(&raw).unwrap_or_default() {
            self.end_pact(ordinal, true, read);
        }
        Ok(())
    }

    /// Ends every present pact, or those naming one of `actors` (ascending).
    fn end_pacts(&mut self, actors: Option<&[u64]>, bounded: bool) -> io::Result<()> {
        let mut ending: Vec<u64> = self
            .present
            .iter()
            .filter(|(signature, _)| {
                actors.is_none_or(|actors| {
                    signature
                        .roles
                        .iter()
                        .any(|role| actors.binary_search(role).is_ok())
                })
            })
            .map(|(_, &ordinal)| ordinal)
            .collect();
        ending.sort_unstable();
        let mut owners = Vec::new();
        for &ordinal in &ending {
            let owner = self.pacts[&ordinal].signature.raw[0];
            let read = self.last_read(owner)?;
            self.end_pact(ordinal, bounded, read);
            owners.push(owner);
        }
        owners.sort_unstable();
        owners.dedup();
        for owner in owners {
            if let Some(held) = self.by_owner.get_mut(&owner) {
                held.retain(|ordinal| ending.binary_search(ordinal).is_err());
                if held.is_empty() {
                    self.by_owner.remove(&owner);
                }
            }
        }
        Ok(())
    }

    fn mark_proposal(dirty: &mut Vec<u64>, key: u64, proposal: &mut Proposal) {
        if !std::mem::replace(&mut proposal.dirty, true) {
            dirty.push(key);
        }
    }

    /// Leaves a present proposal awaiting a fresh read without its owner's handle, in the
    /// group of every proposal that handle left so: that read is when they are all gone by.
    fn suspend(&mut self, key: u64) -> io::Result<()> {
        let proposal = self.proposals.get_mut(&key).expect("present proposal");
        proposal.present = false;
        proposal.bound = true;
        let handle = (proposal.signature.raw[0], proposal.signature.handle);
        let group = match self.awaiting.get(&handle) {
            Some(&group) => group,
            None => {
                let group = self.next_group;
                scalar_id(GONE, group, RETAINED)?;
                self.next_group += 1;
                self.awaiting.insert(handle, group);
                group
            }
        };
        proposal.group = Some(group);
        Self::mark_proposal(&mut self.dirty_proposals, key, proposal);
        Ok(())
    }

    fn suspend_queue(&mut self) -> io::Result<()> {
        self.numbered.clear();
        let mut present: Vec<u64> = self
            .proposals
            .iter()
            .filter(|(_, proposal)| proposal.present)
            .map(|(&key, _)| key)
            .collect();
        present.sort_unstable();
        for key in present {
            self.suspend(key)?;
        }
        self.queued.clear();
        self.queue_read = None;
        Ok(())
    }

    fn end_queue_identities(&mut self, actors: &[u64]) -> io::Result<()> {
        self.numbered.clear();
        let mut ending: Vec<u64> = self
            .proposals
            .iter()
            .filter(|(_, proposal)| {
                proposal.present
                    && proposal
                        .signature
                        .roles
                        .iter()
                        .any(|role| actors.binary_search(role).is_ok())
            })
            .map(|(&key, _)| key)
            .collect();
        ending.sort_unstable();
        for key in ending {
            self.suspend(key)?;
            let signature = self.proposals[&key].signature;
            self.queued.remove(&signature);
        }
        Ok(())
    }

    fn queue(&mut self, writer: &mut Writer, at: i32, queue: QueueRead) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("emit", Some("diplomacy_state.queue"));
        match queue {
            QueueRead::Unavailable => {
                self.suspend_queue()?;
                self.number(writer, 2, at, None)
            }
            QueueRead::Carried => {
                let read = self.queue_read.map(i128::from);
                self.number(writer, 2, at, read)
            }
            QueueRead::Fresh(queue) => self.observe_queue(writer, at, &queue),
        }
    }

    fn observe_queue(&mut self, writer: &mut Writer, at: i32, queue: &Queue) -> io::Result<()> {
        let duplicates = queue.duplicates;
        self.number(
            writer,
            3,
            at,
            (duplicates > 0).then_some(duplicates as i128),
        )?;
        if duplicates > 0 {
            self.suspend_queue()?;
            return self.number(writer, 2, at, None);
        }
        let mut numbered = FastMap::default();
        numbered.reserve(queue.rows.len());
        for (&wire, proposal) in &queue.rows {
            let key = match self.numbered.get(&wire) {
                Some((held, key)) if Arc::ptr_eq(held, proposal) => *key,
                _ => self.begin_proposal(at, proposal)?,
            };
            let held = self.proposals.get_mut(&key).expect("open proposal");
            if held.found == Some(at) {
                return Err(io::Error::other(
                    "Duplicate incoming diplomatic action handle in one owner queue",
                ));
            }
            held.found = Some(at);
            held.last_seen = at;
            if held.last_command != proposal.row.last_command_hour {
                held.last_command = proposal.row.last_command_hour;
                Self::mark_proposal(&mut self.dirty_proposals, key, held);
            }
            numbered.insert(wire, (Arc::clone(proposal), key));
        }
        let mut missing: Vec<u64> = self
            .proposals
            .iter()
            .filter(|(_, proposal)| proposal.present && proposal.found != Some(at))
            .map(|(&key, _)| key)
            .collect();
        missing.sort_unstable();
        for key in missing {
            let proposal = self.proposals.get_mut(&key).expect("missing proposal");
            proposal.gone_by = Some(at);
            proposal.present = false;
            Self::mark_proposal(&mut self.dirty_proposals, key, proposal);
            if self.queued.get(&proposal.signature) == Some(&key) {
                self.queued.remove(&proposal.signature);
            }
        }
        // A reused handle seen after a gap is not an absent row: what awaits it stays
        // unknown until a read without it.
        let mut gone: Vec<((u32, u64), u64)> = self
            .awaiting
            .iter()
            .filter(|(handle, _)| !queue.handles.contains_key(handle))
            .map(|(&handle, &group)| (handle, group))
            .collect();
        gone.sort_unstable_by_key(|&(_, group)| group);
        for (handle, group) in gone {
            self.awaiting.remove(&handle);
            self.gone.push((group, at));
        }
        self.numbered = numbered;
        self.queue_read = Some(at);
        self.number(writer, 2, at, Some(i128::from(at)))
    }

    fn begin_proposal(&mut self, at: i32, proposal: &facts::Proposal) -> io::Result<u64> {
        let named = &self.named;
        let signature = QueueSignature {
            handle: proposal.row.handle,
            raw: proposal.raw(),
            roles: proposal
                .roles
                .map(|index| index.map_or(NOBODY, |index| named[index as usize])),
            stored: proposal.row.hour,
        };
        if let Some(&key) = self.queued.get(&signature) {
            return Ok(key);
        }
        let key = self.next_proposal;
        scalar_id(PROPOSALS, key, GONE)?;
        self.next_proposal += 1;
        self.proposals.insert(
            key,
            Proposal {
                signature,
                created: at,
                last_command: proposal.row.last_command_hour,
                first_seen: at,
                last_seen: at,
                gone_by: None,
                bound: false,
                present: true,
                group: None,
                found: None,
                dirty: true,
            },
        );
        self.dirty_proposals.push(key);
        self.queued.insert(signature, key);
        Ok(key)
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the records that changed.
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
        let mut kinds = std::mem::take(&mut self.dirty_kinds);
        kinds.sort_unstable();
        kinds.dedup();
        for id in kinds {
            let (kind, token) = &self.kind_list[id as usize];
            let text = kind.text();
            let cells = [
                Cell::Text(writer.blob(&mut text.as_bytes(), text.len() as u64)?),
                Cell::Unsigned(u64::from(*token)),
                Cell::Boolean(matches!(kind, Kind::Unresolved(_))),
            ];
            let cell = Cell::List(writer.cells(cells)?);
            writer.catalogue(KINDS + u64::from(id), cell)?;
        }
        let mut dirty = std::mem::take(&mut self.dirty_pacts);
        dirty.sort_unstable();
        for ordinal in dirty {
            let pact = self.pacts.get_mut(&ordinal).expect("dirty pact");
            pact.dirty = false;
            let cell = pact_cell(writer, pact)?;
            writer.catalogue(PACTS + ordinal, cell)?;
            if pact.ended.is_some() {
                self.pacts.remove(&ordinal);
            }
        }
        let mut dirty = std::mem::take(&mut self.dirty_proposals);
        dirty.sort_unstable();
        for key in dirty {
            let proposal = self.proposals.get_mut(&key).expect("dirty proposal");
            proposal.dirty = false;
            let cell = proposal_cell(writer, proposal)?;
            writer.catalogue(PROPOSALS + key, cell)?;
            // Once not present, only its group's read changes it again.
            if !proposal.present {
                self.proposals.remove(&key);
            }
        }
        for (group, hour) in std::mem::take(&mut self.gone) {
            writer.catalogue(GONE + group, Cell::Integer(i128::from(hour)))?;
        }
        let mut changed = std::mem::take(&mut self.retained_changed);
        changed.sort_unstable();
        changed.dedup();
        for actor in changed {
            let cell = self
                .retained
                .get(&actor)
                .map_or(Cell::Null, |&value| Cell::Integer(value));
            writer.catalogue(RETAINED + actor, cell)?;
        }
        let mut reads: Vec<(u32, i32)> = self
            .by_owner
            .keys()
            .map(|&raw| Ok((raw, self.last_read(raw)?)))
            .collect::<io::Result<_>>()?;
        reads.sort_unstable();
        if reads != self.written_reads {
            let cells: Vec<Cell> = reads
                .iter()
                .flat_map(|&(raw, read)| {
                    [Cell::Unsigned(u64::from(raw)), Cell::Integer(i128::from(read))]
                })
                .collect();
            let cell = Cell::List(writer.cells(cells)?);
            writer.catalogue(RAW_READS, cell)?;
            self.written_reads = reads;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn held_proposals(&self) -> usize {
        self.proposals.len()
    }

    #[cfg(test)]
    pub(super) fn open_runs(&self) -> usize {
        self.carry.len()
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection names its countries anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let mut owners: Vec<SavedOwner> = self
            .owners
            .iter()
            .map(|(&actor, owner)| SavedOwner {
                actor,
                read: owner.read,
                known: owner.known,
                unresolved: owner.unresolved,
                pairs: owner
                    .pairs
                    .iter()
                    .map(|pair| (pair.target, pair.value))
                    .collect(),
            })
            .collect();
        owners.sort_unstable_by_key(|owner| owner.actor);
        let sorted = |map: &FastMap<u64, i32>| {
            let mut held: Vec<(u64, i32)> = map.iter().map(|(&key, &read)| (key, read)).collect();
            held.sort_unstable();
            held
        };
        let mut raw_reads: Vec<(u32, i32)> = self
            .raw_reads
            .iter()
            .map(|(&raw, held)| (raw, held.read))
            .collect();
        raw_reads.sort_unstable();
        let mut pacts: Vec<(u64, Pact)> = self
            .pacts
            .iter()
            .map(|(&ordinal, pact)| (ordinal, pact.clone()))
            .collect();
        pacts.sort_unstable_by_key(|&(ordinal, _)| ordinal);
        let mut proposals: Vec<(u64, Proposal)> = self
            .proposals
            .iter()
            .map(|(&key, proposal)| (key, proposal.clone()))
            .collect();
        proposals.sort_unstable_by_key(|&(key, _)| key);
        let saved = Saved {
            last: self.last,
            segment_first: self.segment_first,
            live: self.live.clone(),
            identities: self
                .live
                .iter()
                .map(|&actor| self.countries[actor as usize].identity_first)
                .collect(),
            owners,
            orphan_opinions: self.orphan_opinions,
            tuple_reads: sorted(&self.tuple_reads),
            raw_reads,
            pacts,
            next_pact: self.next_pact,
            proposals,
            queued: {
                let mut queued: Vec<u64> = self.queued.values().copied().collect();
                queued.sort_unstable();
                queued
            },
            next_proposal: self.next_proposal,
            queue_read: self.queue_read,
            awaiting: {
                let mut awaiting: Vec<((u32, u64), u64)> = self
                    .awaiting
                    .iter()
                    .map(|(&handle, &group)| (handle, group))
                    .collect();
                awaiting.sort_unstable();
                awaiting
            },
            next_group: self.next_group,
            written_reads: self.written_reads.clone(),
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
        emit.countries = vec![Country::default(); directory.actors.len()];
        for (actor, country) in emit.countries.iter_mut().enumerate() {
            for (measure, key) in country.keys.iter_mut().enumerate() {
                *key = directory
                    .key(actor as u64, measure as u64, None)
                    .ok_or_else(invalid)?;
            }
            if let Some(value) = directory
                .scalar(RETAINED + actor as u64)
                .and_then(Scalar::integer)
            {
                emit.retained.insert(actor as u64, value);
            }
        }
        let mut keys = [0; 5];
        for (at, key) in keys.iter_mut().enumerate() {
            *key = directory
                .key(FAMILY, OPINION_COVERAGE + at as u64, None)
                .ok_or_else(invalid)?;
        }
        emit.family = Some(keys);
        let mut id = 0;
        while let Some(scalar) = directory.scalar(KINDS + u64::from(id)) {
            let [text, token, unresolved] = scalar.list() else {
                return Err(invalid());
            };
            let token = token
                .integer()
                .and_then(|token| u32::try_from(token).ok())
                .ok_or_else(invalid)?;
            let kind = match unresolved {
                Scalar::Boolean(true) => Kind::Unresolved(token),
                Scalar::Boolean(false) => Kind::Spelled(Arc::from(
                    std::str::from_utf8(text.text().ok_or_else(invalid)?)
                        .map_err(|_| invalid())?,
                )),
                _ => return Err(invalid()),
            };
            emit.kinds.insert(kind.clone(), id);
            emit.kind_list.push((kind, token));
            id += 1;
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.segment_first = saved.segment_first;
        if saved.live.len() != saved.identities.len() {
            return Err(invalid());
        }
        for (&actor, &first) in saved.live.iter().zip(&saved.identities) {
            emit.countries
                .get_mut(actor as usize)
                .ok_or_else(invalid)?
                .identity_first = first;
        }
        emit.live = saved.live;
        for owner in saved.owners {
            if let Some(read) = owner.read {
                emit.opinion_deadlines.insert(read, owner.actor);
            }
            let mut pairs = Vec::with_capacity(owner.pairs.len());
            for (target, value) in owner.pairs {
                let key = directory
                    .key(owner.actor, OPINION, Some(target))
                    .ok_or_else(invalid)?;
                pairs.push(Pair { target, key, value });
            }
            emit.owners_unresolved += owner.unresolved;
            emit.owners.insert(
                owner.actor,
                Owner {
                    read: owner.read,
                    known: owner.known,
                    unresolved: owner.unresolved,
                    values: None,
                    pairs,
                },
            );
        }
        emit.orphan_opinions = saved.orphan_opinions;
        for (actor, read) in saved.tuple_reads {
            emit.tuple_deadlines.insert(read, actor);
            emit.tuple_reads.insert(actor, read);
        }
        for (raw, read) in saved.raw_reads {
            emit.raw_deadlines.insert(read, raw);
            emit.raw_reads.insert(raw, RawRead { read, memo: None });
        }
        for (ordinal, pact) in saved.pacts {
            if pact.ended.is_some() {
                return Err(invalid());
            }
            emit.present.insert(pact.signature, ordinal);
            emit.by_owner
                .entry(pact.signature.raw[0])
                .or_default()
                .push(ordinal);
            emit.pacts.insert(ordinal, pact);
        }
        for held in emit.by_owner.values_mut() {
            held.sort_unstable();
        }
        emit.next_pact = saved.next_pact;
        for (key, proposal) in saved.proposals {
            if !proposal.present {
                return Err(invalid());
            }
            emit.proposals.insert(key, proposal);
        }
        for key in saved.queued {
            let signature = emit.proposals.get(&key).ok_or_else(invalid)?.signature;
            emit.queued.insert(signature, key);
        }
        emit.next_proposal = saved.next_proposal;
        emit.queue_read = saved.queue_read;
        emit.awaiting = saved.awaiting.into_iter().collect();
        emit.next_group = saved.next_group;
        emit.written_reads = saved.written_reads;
        Ok(emit)
    }
}

/// A pact's record: its first hour, first read, kind, ordinal, start, end, raw cancel and
/// retired bytes, raw endpoints, resolved roles, state (0 present, 1 gone from a read, 2 ended
/// bounded) and, once ended, its last read.
fn pact_cell(writer: &mut Writer, pact: &Pact) -> io::Result<Cell> {
    let signature = &pact.signature;
    let mut cells = Vec::with_capacity(18);
    cells.extend([
        Cell::Integer(i128::from(pact.created)),
        Cell::Integer(i128::from(pact.first_seen)),
        Cell::Unsigned(u64::from(signature.kind)),
        Cell::Unsigned(u64::from(signature.ordinal)),
        Cell::Integer(i128::from(signature.start)),
        Cell::Integer(i128::from(pact.end)),
        Cell::Unsigned(u64::from(pact.cancel)),
        Cell::Unsigned(u64::from(pact.retired)),
    ]);
    cells.extend(signature.raw.map(|raw| Cell::Unsigned(u64::from(raw))));
    cells.extend(signature.roles.map(role));
    let (state, last) = match pact.ended {
        None => (0, Cell::Null),
        Some((read, bounded)) => (1 + u64::from(bounded), Cell::Integer(i128::from(read))),
    };
    cells.extend([Cell::Unsigned(state), last]);
    Ok(Cell::List(writer.cells(cells)?))
}

/// A proposal's record: its first hour, handle, raw roles, resolved roles, stored hour, last
/// command, first and last reads, the read it was gone by, whether that is only a bound,
/// whether it is present, and the group whose read it is gone by ([`GONE`]).
fn proposal_cell(writer: &mut Writer, proposal: &Proposal) -> io::Result<Cell> {
    let signature = &proposal.signature;
    let mut cells = Vec::with_capacity(20);
    cells.extend([
        Cell::Integer(i128::from(proposal.created)),
        Cell::Unsigned(signature.handle),
    ]);
    cells.extend(signature.raw.map(|raw| Cell::Unsigned(u64::from(raw))));
    cells.extend(signature.roles.map(role));
    cells.extend([
        Cell::Integer(i128::from(signature.stored)),
        Cell::Integer(i128::from(proposal.last_command)),
        Cell::Integer(i128::from(proposal.first_seen)),
        Cell::Integer(i128::from(proposal.last_seen)),
        proposal
            .gone_by
            .map_or(Cell::Null, |hour| Cell::Integer(i128::from(hour))),
        Cell::Boolean(proposal.bound),
        Cell::Boolean(proposal.present),
        proposal.group.map_or(Cell::Null, Cell::Unsigned),
    ]);
    Ok(Cell::List(writer.cells(cells)?))
}

fn role(actor: u64) -> Cell {
    if actor == NOBODY {
        Cell::Null
    } else {
        Cell::Unsigned(actor)
    }
}

#[derive(Serialize, Deserialize)]
struct SavedOwner {
    actor: u64,
    read: Option<i32>,
    known: bool,
    unresolved: usize,
    pairs: Vec<(u64, i32)>,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    segment_first: Option<i32>,
    live: Vec<u64>,
    /// Each live country's identity run's first hour, in `live`'s order.
    identities: Vec<Option<i32>>,
    owners: Vec<SavedOwner>,
    orphan_opinions: usize,
    tuple_reads: Vec<(u64, i32)>,
    raw_reads: Vec<(u32, i32)>,
    pacts: Vec<(u64, Pact)>,
    next_pact: u64,
    proposals: Vec<(u64, Proposal)>,
    /// The proposals the queue's next read finds by signature.
    queued: Vec<u64>,
    next_proposal: u64,
    queue_read: Option<i32>,
    awaiting: Vec<((u32, u64), u64)>,
    next_group: u64,
    written_reads: Vec<(u32, i32)>,
}
