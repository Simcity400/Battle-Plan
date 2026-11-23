use super::{Captured, RelationRow, valid_span};
use crate::{
    country_input_changes::{self as changes, Changes, Counts, Table},
    table_capture::Arena,
};
use engine_allocation as allocation;
use engine_native_memory::{Memory, cpdx_array::Array, walk::Outcome};
use engine_native::ccountry::{self, CounterInputsRow, RosterSpan};

const WORDS: usize = ccountry::PRODUCTION_STATUS;
const WORDS_LEN: usize = ccountry::CHARACTERS + 8 - WORDS;
const WORD_OFFSETS: [usize; 3] = [
    ccountry::PRODUCTION_STATUS,
    ccountry::DIPLOMACY_STATUS,
    ccountry::CHARACTERS,
];

#[derive(Clone, Copy)]
struct Entry {
    country: engine_native_memory::country_identity::Country,
    words: [u64; 3],
    row: CounterInputsRow,
    relation: RelationRow,
}

#[derive(Default)]
struct Held {
    epoch: Option<u64>,
    arrays: [Array; 2],
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

#[derive(Default)]
struct Hour {
    epoch: Option<u64>,
    arrays: Option<[Array; 2]>,
    complete: bool,
    certified: bool,
}

#[derive(Clone, Copy)]
struct Walked {
    country: engine_native_memory::country_identity::Country,
    words: Option<[u64; 3]>,
    carried: Option<usize>,
}

pub struct CounterStore {
    changes: &'static dyn Changes,
    held: Held,
    hour: Hour,
    forgotten: bool,
    bytes: usize,
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

    pub fn observed() -> Self {
        Self::new(&changes::Observer)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    pub fn forget(&mut self) {
        self.forgotten = true;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

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
        let arrays = ccountry::Reader::new(memory, base, game_state)
            .ok()
            .map(|reader| reader.arrays());
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

    pub(super) fn offer(
        &mut self,
        memory: &impl Memory,
        country: engine_native_memory::country_identity::Country,
        rosters: &mut ccountry::CounterRosters<'_>,
        effects: &mut ccountry::EffectRosters<'_>,
    ) -> Option<CounterInputsRow> {
        if self.walked.len() == self.walked.capacity() {
            self.hour.complete = false;
            self.hour.certified = false;
            return None;
        }
        let words = country
            .pointer
            .checked_add(WORDS)
            .and_then(|at| engine_native_memory::read::<[u8; WORDS_LEN]>(memory, at))
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

    fn certified_entry(
        &self,
        country: engine_native_memory::country_identity::Country,
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

fn arena_len<T>(arena: &Arena<T>) -> Option<usize> {
    (arena.len <= arena.first.len()).then_some(arena.len)
}

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
