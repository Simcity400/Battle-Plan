use super::*;
use dashboard_text::triggers::inputs::{
    self, Resolution, Since, TableChange, TableChanges, TableRead,
};
use replay_frame::frame::HourFrame;

pub(super) type Key = (String, u32, u32);

/// A law's answer, what it read, and what its content calls noted.
type Evaluated = (
    (Result<bool, String>, inputs::Inputs),
    catalogue::OwnRefusals,
);

fn evaluated<'w>(
    frame: HourFrame<'w>,
    evaluate: &impl Fn(HourFrame<'w>) -> Result<bool, String>,
) -> Evaluated {
    catalogue::capture_isolated(|| inputs::capture_in(frame, || evaluate(frame)))
}

#[derive(Default)]
struct Ranges {
    prefixes: HashMap<u32, HashMap<u128, HashSet<TableRead>>>,
    owners: HashMap<(u32, Resolution), HashMap<u32, HashSet<TableRead>>>,
    broad: HashSet<TableRead>,
}

fn prefix(read: &TableRead) -> Option<(u32, u128)> {
    let (first, last) = read.rows?;
    let different = first ^ last;
    let shift = different.trailing_ones();
    if shift >= 128 {
        return None;
    }
    let mask = (1_u128 << shift) - 1;
    (different == mask && first & mask == 0).then_some((shift, first >> shift))
}

impl Ranges {
    fn insert(&mut self, read: TableRead) {
        if let Some(owner) = read.owner {
            self.owners
                .entry((owner.shift, owner.by))
                .or_default()
                .entry(owner.slot)
                .or_default()
                .insert(read);
        } else if let Some((shift, key)) = prefix(&read) {
            self.prefixes
                .entry(shift)
                .or_default()
                .entry(key)
                .or_default()
                .insert(read);
        } else {
            self.broad.insert(read);
        }
    }

    fn remove(&mut self, read: &TableRead) {
        if let Some(owner) = read.owner {
            let by = (owner.shift, owner.by);
            if let Some(owners) = self.owners.get_mut(&by)
                && let Some(reads) = owners.get_mut(&owner.slot)
            {
                reads.remove(read);
                if reads.is_empty() {
                    owners.remove(&owner.slot);
                }
                if owners.is_empty() {
                    self.owners.remove(&by);
                }
            }
        } else if let Some((shift, key)) = prefix(read) {
            if let Some(prefixes) = self.prefixes.get_mut(&shift)
                && let Some(reads) = prefixes.get_mut(&key)
            {
                reads.remove(read);
                if reads.is_empty() {
                    prefixes.remove(&key);
                }
                if prefixes.is_empty() {
                    self.prefixes.remove(&shift);
                }
            }
        } else {
            self.broad.remove(read);
        }
    }

    fn reached(
        &self,
        frame: HourFrame<'_>,
        changed: &[replay_data::rows::DeltaRow],
        deleted: &[u128],
    ) -> HashSet<TableRead> {
        let mut reads = self.broad.clone();
        let links = Resolution::Links.slots(frame);
        let countries = Resolution::Countries.slots(frame);
        for key in changed
            .iter()
            .map(|row| row.key)
            .chain(deleted.iter().copied())
        {
            for (&shift, prefixes) in &self.prefixes {
                if let Some(found) = prefixes.get(&(key >> shift)) {
                    reads.extend(found.iter().copied());
                }
            }
            for (&(shift, by), owners) in &self.owners {
                let raw = key.checked_shr(shift).unwrap_or(0) as u32;
                let slots = match by {
                    Resolution::Links => links,
                    Resolution::Countries => countries,
                };
                let slot = slots.and_then(|slots| slots.of(raw));
                match slot {
                    Some(slot) => {
                        if let Some(found) = owners.get(&slot) {
                            reads.extend(found.iter().copied());
                        }
                    }
                    None => {
                        reads.extend(owners.values().flat_map(|reads| reads.iter().copied()));
                    }
                }
            }
        }
        reads
    }
}

struct Evaluation {
    value: Result<bool, String>,
    own: catalogue::OwnRefusals,
    reads: Vec<TableRead>,
    dirty: bool,
    states: BTreeSet<u32>,
    /// Which evaluation of its runtime it is.
    generation: u64,
}

#[derive(Default)]
pub(super) struct Runtime {
    values: HashMap<Key, Evaluation>,
    watches: HashMap<TableId, HashMap<TableRead, BTreeSet<Key>>>,
    ranges: HashMap<TableId, Ranges>,
    ending: HashSet<TableId>,
    volatile: BTreeSet<Key>,
    states: HashMap<u32, Key>,
    at: Option<(u64, i32)>,
    /// Evaluations made, never reset: a generation names one evaluation only.
    generations: u64,
}

impl Runtime {
    pub(super) fn clear(&mut self) {
        let generations = self.generations;
        *self = Self::default();
        self.generations = generations;
    }

    /// The evaluation `state`'s law read last, and its generation.
    pub(super) fn read_by(&self, state: u32) -> Option<(Key, u64)> {
        let key = self.states.get(&state)?;
        Some((key.clone(), self.values.get(key)?.generation))
    }

    /// Whether `key`'s answer is still that of evaluation `generation`, and none of its reads
    /// changed since: reading it again would remember it.
    pub(super) fn current(&self, key: &Key, generation: u64) -> bool {
        !inputs::check_reuse()
            && self
                .values
                .get(key)
                .is_some_and(|value| !value.dirty && value.generation == generation)
    }

    /// What remembering `key`'s answer for `state` does ([`Self::evaluate`]) once it is
    /// [`Self::current`]: `state` reads it, and what its content calls noted is credited.
    pub(super) fn remember(&mut self, state: u32, key: &Key) {
        self.forget_state(state);
        self.states.insert(state, key.clone());
        let value = self.values.get_mut(key).expect("a current law evaluation");
        value.states.insert(state);
        catalogue::credit(&value.own);
    }

    pub(super) fn forget_state(&mut self, state: u32) {
        if let Some(key) = self.states.remove(&state)
            && let Some(value) = self.values.get_mut(&key)
        {
            value.states.remove(&state);
        }
    }

    pub(super) fn step(
        &mut self,
        frame: HourFrame<'_>,
        changes: &dyn TableChanges,
    ) -> BTreeSet<u128> {
        let at = (frame.branch(), frame.hour());
        if self
            .at
            .is_some_and(|(branch, hour)| branch != at.0 || hour.checked_add(1) != Some(at.1))
        {
            let states = self.states.keys().map(|&id| u128::from(id)).collect();
            self.clear();
            self.at = Some(at);
            return states;
        }
        self.at = Some(at);
        let mut dirty = self.volatile.clone();
        if let Some(since) = Since::hour_before_in(frame, changes) {
            for (&table, reads) in &self.watches {
                if matches!(changes.table(table), TableChange::Same)
                    && !self.ending.contains(&table)
                {
                    continue;
                }
                let candidates: HashSet<_> = match changes.table(table) {
                    TableChange::Rows(changed, deleted) => {
                        self.ranges.get(&table).map_or_else(HashSet::new, |ranges| {
                            ranges.reached(frame, changed, deleted)
                        })
                    }
                    TableChange::Same => HashSet::new(),
                    TableChange::Fields(fields) => reads
                        .keys()
                        .filter(|read| read.fields & fields != 0)
                        .copied()
                        .collect(),
                    TableChange::Any => reads.keys().copied().collect(),
                };
                let mut candidates = candidates;
                if self.ending.contains(&table) {
                    candidates.extend(reads.keys().filter(|read| read.ends).copied());
                }
                for read in candidates {
                    if since.changed(&read)
                        && let Some(keys) = reads.get(&read)
                    {
                        dirty.extend(keys.iter().cloned());
                    }
                }
            }
        } else {
            dirty.extend(self.values.keys().cloned());
        }
        let mut states = BTreeSet::new();
        for key in dirty {
            if let Some(value) = self.values.get_mut(&key) {
                value.dirty = true;
                states.extend(value.states.iter().map(|&id| u128::from(id)));
            }
        }
        states
    }

    /// The law `token`'s eligibility for `controller` over `occupied`, which
    /// `evaluate` decides from a trigger world read from `frame`, holding every trigger
    /// table: the remembered answer while none of its declared reads changed. Check mode
    /// evaluates a remembered answer anyway, and a change is a missed declaration
    /// ([`inputs::missed_reuse_at`]).
    pub(super) fn evaluate<'w>(
        &mut self,
        frame: HourFrame<'w>,
        state: u32,
        token: &str,
        controller: u32,
        occupied: u32,
        evaluate: impl Fn(HourFrame<'w>) -> Result<bool, String>,
    ) -> Result<bool, String> {
        let key = (token.to_owned(), controller, occupied);
        if let Some(value) = self.values.get(&key).filter(|value| !value.dirty) {
            if inputs::check_reuse() {
                let ((checked, _), _) = evaluated(frame, &evaluate);
                if checked != value.value {
                    inputs::missed_reuse_at(&("occupation law", &key), frame.hour());
                }
            }
        } else {
            let mut states = BTreeSet::new();
            if let Some(old) = self.values.remove(&key) {
                states = old.states;
                for read in old.reads {
                    if let Some(reads) = self.watches.get_mut(&read.table)
                        && let Some(keys) = reads.get_mut(&read)
                    {
                        keys.remove(&key);
                        if keys.is_empty() {
                            reads.remove(&read);
                            if let Some(ranges) = self.ranges.get_mut(&read.table) {
                                ranges.remove(&read);
                            }
                        }
                        if !reads.keys().any(|read| read.ends) {
                            self.ending.remove(&read.table);
                        }
                    }
                }
                self.volatile.remove(&key);
            }
            let ((value, inputs), own) = evaluated(frame, &evaluate);
            let volatile = inputs.declared().is_none();
            let mut reads = inputs.declared().unwrap_or_default().to_vec();
            if reads.iter().any(|read| read.owner.is_some()) {
                let links = TableRead::fields(TableId::CountryLinks, &["canonical_slot"]);
                if !reads.contains(&links) {
                    reads.push(links);
                }
            }
            for &read in &reads {
                if read.ends {
                    self.ending.insert(read.table);
                }
                self.ranges.entry(read.table).or_default().insert(read);
                self.watches
                    .entry(read.table)
                    .or_default()
                    .entry(read)
                    .or_default()
                    .insert(key.clone());
            }
            if volatile {
                self.volatile.insert(key.clone());
            }
            self.generations += 1;
            self.values.insert(
                key.clone(),
                Evaluation {
                    value,
                    own,
                    reads,
                    dirty: false,
                    states,
                    generation: self.generations,
                },
            );
        }
        self.forget_state(state);
        self.states.insert(state, key.clone());
        let value = self.values.get_mut(&key).expect("evaluated law");
        value.states.insert(state);
        catalogue::credit(&value.own);
        value.value.clone()
    }
}
