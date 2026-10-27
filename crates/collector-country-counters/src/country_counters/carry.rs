//! CountryCounterInputs rows held between hours
//! ([`crate::country_input_changes::Table::Counters`]).
//!
//! The Ordinary counter part walks the country census every hour. A
//! country's counter row (pride of the fleet, attack modifiers, state counts,
//! first division, owned/controlled/core states, army and navy leaders,
//! neighbours and MIO policies) and its relation suffix (co-belligerents,
//! zero war, zero subject and forward visibility) are read afresh only when a
//! hooked writer marked it since the previous take, it is new to the census
//! or its last read was not held. Every other country's row is copied from
//! its held read, with its roster and co-belligerent entries placed in the
//! hour's arenas where a full read puts them
//! ([`ccountry::CarriedRosters::append`]), so the published table equals a
//! full read of the same memory.
//!
//! A country is carried only while all of these hold: the hour's take returned the epoch its held
//! read was certified under; it is not marked; the census country and link headers are the held
//! ones and the census names the held object and native id at its canonical slot; its production
//! status, diplomacy status and country characters (CCountryCharacters) pointers, read for every
//! country each hour in one guarded copy, are the held ones. Its identity (vtable and `+ID`) is
//! still checked before the carry, as before any read.
//!
//! Every hour still reads, for every country, carried or not: the pride ship
//! name behind a nonzero pride handle (no hookable writer per country), the
//! census resolution of its id and its cell of the reserved country's reverse
//! visibility matrix (with no forward observation to share).
//!
//! A sent-reference refusal is held with the row; other required read
//! refusals prevent holding it (a native null production status is not a
//! refusal). Rows are held only from a complete walk and relation pass under
//! a take that returned an epoch: a failed or incomplete read, or a failed
//! job, holds nothing, so the next hour reads every country. Storage is
//! advisory and worker-only.
use super::{Captured, RelationRow, valid_span};
use crate::{
    country_input_changes::{self as changes, Changes, Counts, Table},
    table_capture::Arena,
};
use engine_allocation as allocation;
use engine_native::{
    Memory, ccountry, ccountry::CounterInputsRow, ccountry::RosterSpan, cpdx_array::Array,
    walk::Outcome,
};

/// The first of the country words a carry certifies; one guarded copy of
/// [`WORDS_LEN`] bytes covers all three.
const WORDS: usize = ccountry::PRODUCTION_STATUS;
const WORDS_LEN: usize = ccountry::CHARACTERS + 8 - WORDS;
/// The production status pointer holds production status and its policy roster for MIO policies;
/// the diplomacy status pointer holds diplomacy status, whose relation matrix and rebuilt wartime
/// ally candidates the relation suffix reads; the country characters pointer holds the country
/// character pool, through which the army and navy leader lookups reach rosters (the leader
/// rosters).
const WORD_OFFSETS: [usize; 3] = [
    ccountry::PRODUCTION_STATUS,
    ccountry::DIPLOMACY_STATUS,
    ccountry::CHARACTERS,
];

/// One country's held read.
#[derive(Clone, Copy)]
struct Entry {
    country: ccountry::Country,
    words: [u64; 3],
    /// Spans name the held arenas.
    row: CounterInputsRow,
    /// The co-belligerent span names the held arena; pride fields unused.
    relation: RelationRow,
}

/// The latest held complete read.
#[derive(Default)]
struct Held {
    /// The take epoch it was certified under; `None` holds nothing.
    epoch: Option<u64>,
    /// The census country and link headers it walked.
    arrays: [Array; 2],
    /// By canonical slot, ascending.
    entries: Vec<Entry>,
    states: Vec<u32>,
    leaders: Vec<u64>,
    neighbors: Vec<u32>,
    policies: Vec<u32>,
    co_belligerents: Vec<u32>,
}

impl Held {
    fn clear(&mut self) {
        self.epoch = None;
        self.entries.clear();
        self.states.clear();
        self.leaders.clear();
        self.neighbors.clear();
        self.policies.clear();
        self.co_belligerents.clear();
    }

    fn measure(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.states.capacity() * size_of::<u32>()
            + self.leaders.capacity() * size_of::<u64>()
            + self.neighbors.capacity() * size_of::<u32>()
            + self.policies.capacity() * size_of::<u32>()
            + self.co_belligerents.capacity() * size_of::<u32>()
    }
}

/// The hour's view before the walk.
#[derive(Default)]
struct Hour {
    epoch: Option<u64>,
    arrays: Option<[Array; 2]>,
    /// The marks and the walk's scratch are complete: the hour may carry
    /// and hold.
    complete: bool,
    /// Held reads may be carried this hour.
    certified: bool,
}

/// What the hour did with one walked country, in walk (row) order.
#[derive(Clone, Copy)]
struct Walked {
    country: ccountry::Country,
    /// `None`: unreadable, so nothing of this read is held.
    words: Option<[u64; 3]>,
    /// The held entry its row was copied from.
    carried: Option<usize>,
}

/// The Ordinary counter part's held rows and scratch.
pub struct CounterStore {
    changes: &'static dyn Changes,
    held: Held,
    hour: Hour,
    /// Set on the game thread; the next capture drops every held row.
    forgotten: bool,
    /// Allocated bytes as of the last capture, for the game thread.
    bytes: usize,
    /// The hour's marked native ids, ascending.
    marked: Vec<u32>,
    walked: Vec<Walked>,
    counts: Counts,
}

impl CounterStore {
    pub fn new(changes: &'static dyn Changes) -> Self {
        Self {
            changes,
            held: Held::default(),
            hour: Hour::default(),
            forgotten: false,
            bytes: 0,
            marked: Vec::new(),
            walked: Vec::new(),
            counts: Counts::default(),
        }
    }

    /// The hooks' domain.
    pub fn observed() -> Self {
        Self::new(&changes::Observer)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// Nothing held certifies anything: the next capture reads afresh.
    /// Allocation-free for the game thread.
    pub fn forget(&mut self) {
        self.forgotten = true;
    }

    /// As of the last capture, so the game thread need not measure.
    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    /// Worker, before the walk: take the hour's marks and the census
    /// headers that certify carried rows.
    pub(super) fn begin(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        if std::mem::take(&mut self.forgotten) {
            self.held.clear();
        }
        self.counts = Counts::default();
        self.marked.clear();
        self.walked.clear();
        let changes = self.changes;
        let marked = &mut self.marked;
        let mut complete = true;
        let epoch = changes.take(Table::Counters, &mut |id| match u32::try_from(id) {
            Ok(id) if allocation::advisory(|| allocation::reserve(marked, 1)).is_ok() => {
                marked.push(id)
            }
            _ => complete = false,
        });
        self.marked.sort_unstable();
        self.marked.dedup();
        // The walk's own headers: the callback census's when it is reused.
        let arrays = ccountry::Reader::new(memory, base, game_state)
            .ok()
            .map(|reader| reader.arrays());
        // One walked entry per census slot at most, so the walk never grows it.
        let slots = arrays.map_or(0, |[countries, _]| countries.count as usize);
        if allocation::advisory(|| allocation::reserve(&mut self.walked, slots)).is_err() {
            complete = false;
        }
        let certified = complete
            && epoch.is_some()
            && epoch == self.held.epoch
            && arrays == Some(self.held.arrays);
        self.hour = Hour {
            epoch,
            arrays,
            complete,
            certified,
        };
    }

    pub(super) fn has_certificate(&self) -> bool {
        self.hour.epoch.is_some()
    }

    /// The walk offers `country` once its identity is confirmed: its row
    /// from the held read, with its rosters placed in the hour's arenas, or
    /// `None` to read it.
    pub(super) fn offer(
        &mut self,
        memory: &impl Memory,
        country: ccountry::Country,
        rosters: &mut ccountry::CounterRosters<'_>,
        effects: &mut ccountry::EffectRosters<'_>,
    ) -> Option<CounterInputsRow> {
        if self.walked.len() == self.walked.capacity() {
            // Unrecorded: nothing later is carried or held.
            self.hour.complete = false;
            self.hour.certified = false;
            return None;
        }
        let words = country
            .pointer
            .checked_add(WORDS)
            .and_then(|at| engine_native::read::<[u8; WORDS_LEN]>(memory, at))
            .map(|bytes| {
                WORD_OFFSETS.map(|offset| {
                    let at = offset - WORDS;
                    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
                })
            });
        let mut walked = Walked {
            country,
            words,
            carried: None,
        };
        let mut row = None;
        if let Some(index) = self.certified_entry(country, words) {
            row = self.place(index, rosters, effects);
            walked.carried = row.is_some().then_some(index);
        }
        self.walked.push(walked);
        row
    }

    /// The held entry that certifies `country` this hour, when it is not
    /// marked.
    fn certified_entry(
        &self,
        country: ccountry::Country,
        words: Option<[u64; 3]>,
    ) -> Option<usize> {
        if !self.hour.certified {
            return None;
        }
        let words = words?;
        let index = self
            .held
            .entries
            .binary_search_by_key(&country.canonical_slot, |entry| {
                entry.country.canonical_slot
            })
            .ok()?;
        let entry = &self.held.entries[index];
        (entry.country == country
            && entry.words == words
            && self.marked.binary_search(&country.id).is_err())
        .then_some(index)
    }

    /// The held row of entry `index` with its rosters appended to the
    /// hour's arenas; `None`, with nothing appended, when they cannot hold
    /// them.
    fn place(
        &self,
        index: usize,
        rosters: &mut ccountry::CounterRosters<'_>,
        effects: &mut ccountry::EffectRosters<'_>,
    ) -> Option<CounterInputsRow> {
        let held = &self.held;
        let row = held.entries[index].row;
        let [owned, controlled, core] = row.state_rosters?;
        let [army, navy] = row.leader_rosters?;
        let carried = ccountry::CarriedRosters {
            states: [
                span(&held.states, owned)?,
                span(&held.states, controlled)?,
                span(&held.states, core)?,
            ],
            leaders: [span(&held.leaders, army)?, span(&held.leaders, navy)?],
            sent_expeditionaries: match row.sent_expeditionaries {
                Some(source) => Some(span(&held.leaders, source)?),
                None => None,
            },
            neighbors: span(&held.neighbors, row.neighbors?)?,
            policies: match row.mio_policies {
                Some(policies) => Some(span(&held.policies, policies)?),
                None => None,
            },
        };
        let spans = carried.append(rosters, effects)?;
        Some(CounterInputsRow {
            state_rosters: Some(spans.states),
            leader_rosters: Some(spans.leaders),
            sent_expeditionaries: spans.sent_expeditionaries,
            neighbors: Some(spans.neighbors),
            mio_policies: spans.policies,
            ..row
        })
    }

    /// The held relation suffix and co-belligerent targets of row `index`
    /// (country `country`) of a `len`-row walk, when its counter row was
    /// carried.
    pub(super) fn carried_relation(
        &self,
        index: usize,
        len: usize,
        country: u32,
    ) -> Option<(RelationRow, &[u32])> {
        if self.walked.len() != len {
            return None;
        }
        let walked = self.walked.get(index)?;
        let entry = self.held.entries.get(walked.carried?)?;
        if walked.country.id != country {
            return None;
        }
        let targets = span(&self.held.co_belligerents, entry.relation.co_belligerents?)?;
        Some((entry.relation, targets))
    }

    /// Worker, after the relation pass: holds a complete read and records
    /// the hour.
    pub(super) fn finish(&mut self, capture: &Captured) {
        let len = capture.rows.outcome.len();
        let tracked = self.walked.len() == len;
        let carried = self
            .walked
            .iter()
            .filter(|walked| walked.carried.is_some())
            .count();
        self.counts = Counts {
            marked: self.marked.len() as u64,
            read: len.saturating_sub(carried) as u64,
            full: !self.hour.certified,
            ..Counts::default()
        };
        self.hold(capture, tracked);
        let ids = self.hour.arrays.map(|[_, links]| {
            self.walked
                .iter()
                .map(|walked| walked.country.id as usize + 1)
                .max()
                .unwrap_or(0)
                .max(links.count as usize)
        });
        changes::record(Table::Counters, &self.counts, ids);
        self.bytes = self.held.measure()
            + self.marked.capacity() * size_of::<u32>()
            + self.walked.capacity() * size_of::<Walked>();
    }

    /// Holds every complete row of a complete walk and relation pass under
    /// the hour's take; otherwise holds nothing.
    fn hold(&mut self, capture: &Captured, tracked: bool) {
        let len = capture.rows.outcome.len();
        let complete = self.hour.complete
            && tracked
            && len != 0
            && capture.rows.outcome == (Outcome::Rows { len })
            && capture.relations.len == len
            && [
                arena_len(&capture.states),
                arena_len(&capture.leaders),
                arena_len(&capture.neighbors),
                arena_len(&capture.policies),
                arena_len(&capture.co_belligerents),
            ]
            .iter()
            .all(Option::is_some);
        let (Some(epoch), Some(arrays), true) = (self.hour.epoch, self.hour.arrays, complete)
        else {
            self.held.clear();
            return;
        };
        let held = &mut self.held;
        let stored = allocation::advisory(|| {
            held.clear();
            allocation::reserve(&mut held.entries, len)?;
            allocation::reserve(&mut held.states, capture.states.len)?;
            allocation::reserve(&mut held.leaders, capture.leaders.len)?;
            allocation::reserve(&mut held.neighbors, capture.neighbors.len)?;
            allocation::reserve(&mut held.policies, capture.policies.len)?;
            allocation::reserve(&mut held.co_belligerents, capture.co_belligerents.len)
        });
        if stored.is_err() {
            held.clear();
            return;
        }
        held.states
            .extend_from_slice(&capture.states.first[..capture.states.len]);
        held.leaders
            .extend_from_slice(&capture.leaders.first[..capture.leaders.len]);
        held.neighbors
            .extend_from_slice(&capture.neighbors.first[..capture.neighbors.len]);
        held.policies
            .extend_from_slice(&capture.policies.first[..capture.policies.len]);
        held.co_belligerents
            .extend_from_slice(&capture.co_belligerents.first[..capture.co_belligerents.len]);
        for (index, walked) in self.walked.iter().enumerate() {
            let row = capture.rows.rows[index];
            let relation = capture.relations.first[index];
            let Some(words) = walked.words else {
                continue;
            };
            if row.country != walked.country.id
                || !clean(&row, &relation)
                || !within(capture, &row, &relation)
            {
                continue;
            }
            held.entries.push(Entry {
                country: walked.country,
                words,
                row,
                relation: RelationRow {
                    pride_name: None,
                    pride_name_reason: None,
                    ..relation
                },
            });
        }
        held.epoch = Some(epoch);
        held.arrays = arrays;
    }
}

fn span<T>(arena: &[T], span: RosterSpan) -> Option<&[T]> {
    arena.get(span.start..span.start.checked_add(span.count)?)
}

/// The arena's filled entries, when its length is within its storage.
fn arena_len<T>(arena: &Arena<T>) -> Option<usize> {
    (arena.len <= arena.first.len()).then_some(arena.len)
}

/// A row and relation suffix without any read refusal: the only rows held.
/// A null production status is the country's native state, not a refusal.
fn clean(row: &CounterInputsRow, relation: &RelationRow) -> bool {
    let policies = match row.policy_reason {
        None => row.mio_policies.is_some(),
        Some(reason) => reason == ccountry::POLICY_STATUS_NULL && row.mio_policies.is_none(),
    };
    policies
        && row.roster_reason.is_none()
        && row.neighbor_reason.is_none()
        && row.state_rosters.is_some()
        && row.leader_rosters.is_some()
        && row.neighbors.is_some()
        && row.attack_modifiers.is_some()
        && row.movement_modifiers.is_some()
        && row.recruitment_modifiers.is_some()
        && row.air_modifiers.is_some()
        && row.air_combat_modifiers.is_some()
        && row.mines_effect_reduction.is_some()
        && row.naval_detection.is_some()
        && relation.co_reason.is_none()
        && relation.zero_reason.is_none()
        && relation.zero_subject_reason.is_none()
        && relation.forward_visibility_reason.is_none()
        && relation.forward_visibility_failures == [None; 4]
        && relation.co_belligerents.is_some()
        && relation.zero_war.is_some()
        && relation.zero_subject.is_some()
        && relation.forward_visibility.is_some()
}

/// Every span of the row lies within the capture's filled arenas.
fn within(capture: &Captured, row: &CounterInputsRow, relation: &RelationRow) -> bool {
    let states = row.state_rosters.is_some_and(|spans| {
        spans
            .iter()
            .all(|span| valid_span(&capture.states, span.start, span.count))
    });
    let sent = row
        .sent_expeditionaries
        .is_none_or(|span| valid_span(&capture.leaders, span.start, span.count));
    let leaders = row.leader_rosters.is_some_and(|spans| {
        spans
            .iter()
            .all(|span| valid_span(&capture.leaders, span.start, span.count))
    });
    let neighbors = row
        .neighbors
        .is_some_and(|span| valid_span(&capture.neighbors, span.start, span.count));
    let policies = row
        .mio_policies
        .is_none_or(|span| valid_span(&capture.policies, span.start, span.count));
    let co_belligerents = relation
        .co_belligerents
        .is_some_and(|span| valid_span(&capture.co_belligerents, span.start, span.count));
    states && sent && leaders && neighbors && policies && co_belligerents
}

#[cfg(test)]
mod tests;
