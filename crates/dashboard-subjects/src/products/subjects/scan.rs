//! The subject scan the family's emit steps through a branch's hours ([`super::emit`]): what
//! the hourly row scan ([`super::row`]) observes of each hour, from the tables the hour changed
//! ([`Inputs`]). An input read anew equal to the one held changes nothing. An hour that changes
//! nothing the scan reads, continues the hour before and ends no relation is the hour before's
//! again: its diagnostic rates are added and its relations go on. An hour changing only levels
//! visits the relations of the subjects whose level changed. Runs are sealed through the last
//! hour at the next change; the emit reads the relations going on after each hour it observes
//! ([`Scan::continuing`]) and takes the starts it found ([`Scan::take_starts`]), so the scan
//! keeps only its open runs.

use super::{LevelStep, SubjectRun, SubjectStart, holds};
use dashboard_base::products::country_groups::{HourSpan, extend, mark_unobserved};
use dashboard_text::metadata::{CountryKeys, LinkTags};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    mem::size_of,
    sync::Arc,
};

/// What an hour holds of a table the scan reads: what the hour before held, nothing (the table
/// is not complete), or its rows read anew.
#[derive(Clone, Debug, Default)]
#[derive(replay_spool::Spooled)]
pub enum Input<T> {
    #[default]
    Same,
    Incomplete,
    Read(T),
}

/// A relation as the scan reads it: its countries (raw references), start, end and cancel byte.
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Relation {
    pub overlord: u32,
    pub subject: u32,
    pub start_hour: i32,
    pub end_hour: i32,
    pub cancel: u8,
}

impl Relation {
    fn active(&self, hour: i32) -> bool {
        holds(self.cancel, self.start_hour, self.end_hour, hour)
    }
}

/// What an hour holds of the tables the scan reads: the countries' keys in table order, each raw
/// link's canonical slot, the relations in key order and each subject's level in key order.
#[derive(Debug, Default)]
#[derive(replay_spool::Spooled)]
pub struct Inputs {
    pub countries: Input<CountryKeys>,
    pub links: Input<HashMap<u32, u32>>,
    pub relations: Input<Vec<Relation>>,
    pub levels: Input<Vec<(u32, Vec<u8>)>>,
}

impl Inputs {
    /// Whether it reads every table, as a segment's first hour does.
    pub fn whole(&self) -> bool {
        !matches!(self.countries, Input::Same)
            && !matches!(self.links, Input::Same)
            && !matches!(self.relations, Input::Same)
            && !matches!(self.levels, Input::Same)
    }

    /// The bytes it owns on the heap, about.
    pub fn heap(&self) -> usize {
        let countries = match &self.countries {
            Input::Read(keys) => keys
                .iter()
                .map(|(_, _, tag)| size_of::<(u32, u32, Vec<u8>)>() + tag.len())
                .sum(),
            _ => 0,
        };
        let links = match &self.links {
            Input::Read(links) => links.capacity() * 8 / 7 * (size_of::<(u32, u32)>() + 1),
            _ => 0,
        };
        let relations = match &self.relations {
            Input::Read(rows) => rows.capacity() * size_of::<Relation>(),
            _ => 0,
        };
        let levels = match &self.levels {
            Input::Read(rows) => rows
                .iter()
                .map(|(_, state)| size_of::<(u32, Vec<u8>)>() + state.capacity())
                .sum(),
            _ => 0,
        };
        countries + links + relations + levels
    }
}

/// A relation's identity: its countries' (slot, id) and its start hour.
type RelationStart = ((u32, u32), (u32, u32), i32);

/// One branch's subject history as of the last hour stepped.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Scan {
    /// The open runs, by `open`.
    runs: Vec<SubjectRun>,
    /// The last unobserved span: the emit publishes each as it grows.
    unobserved: Vec<HourSpan>,
    /// The starts found since the emit last took them.
    starts: Vec<SubjectStart>,
    observed: bool,
    #[serde(with = "dashboard_core::map_pairs")]
    open: BTreeMap<(String, String, i32), usize>,
    tags: LinkTags,
    unmapped: BTreeSet<u32>,
    incomplete: u32,
    unleveled: u32,
    unreadable: u32,
    conflicting: BTreeSet<String>,
    seen_starts: BTreeSet<RelationStart>,
    named_since: BTreeMap<u32, (String, i32)>,
    continuous_since: Option<i32>,
    first_observed: Option<i32>,
    previous_complete: Option<i32>,
    previous_identities: BTreeMap<u32, u32>,
    identities: BTreeMap<u32, u32>,
    identities_changed: bool,
    identity_since: BTreeMap<u32, (u32, i32)>,
    previous_hour: Option<i32>,
    carried: bool,
    diagnostic_rate: (u32, u32, u32),
    unobserved_here: bool,
    next_expiry: Option<i32>,
    // The inputs as last read: a scan taken up again reads every table anew.
    #[serde(skip)]
    country_keys: Option<CountryKeys>,
    #[serde(skip)]
    links: Option<HashMap<u32, u32>>,
    #[serde(skip)]
    relations: Option<Arc<Vec<Relation>>>,
    #[serde(skip)]
    levels: Option<Vec<(u32, Vec<u8>)>>,
    /// The levels or the tags changed since the levels were last resolved.
    #[serde(skip)]
    relevel: bool,
    /// Whether an hour was observed since the scan began or was taken up: until one is, the
    /// inputs held are unknown, not what the hour before held.
    #[serde(skip)]
    primed: bool,
    #[serde(skip)]
    resolved_levels: Option<Arc<HashMap<String, Option<Vec<u8>>>>>,
    #[serde(skip)]
    relations_by_tag: BTreeMap<String, Vec<usize>>,
}

/// `value` after `hours` additions of `rate`, each made as `+=` makes it: a
/// sum that cannot overflow at once, otherwise one addition at a time, so an
/// overflow behaves exactly as it does hour by hour.
pub(crate) fn repeated(value: u32, rate: u32, hours: u64) -> u32 {
    u32::try_from(hours)
        .ok()
        .and_then(|hours| rate.checked_mul(hours))
        .and_then(|added| value.checked_add(added))
        .unwrap_or_else(|| (0..hours).fold(value, |value, _| value + rate))
}

/// `held` replaced by what `input` reads, and whether that changed it.
fn take<T: PartialEq>(held: &mut Option<T>, input: Input<T>) -> bool {
    match input {
        Input::Same => false,
        Input::Incomplete => held.take().is_some(),
        Input::Read(read) => {
            let changed = held.as_ref() != Some(&read);
            if changed {
                *held = Some(read);
            }
            changed
        }
    }
}

impl Scan {
    /// A branch's scan whose first hour is `first`.
    pub fn starting(first: i32) -> Self {
        Self {
            previous_hour: first.checked_sub(1),
            ..Self::default()
        }
    }

    /// Steps the scan through `hour`; true when it observed the hour anew, false when the hour
    /// was the hour before's again.
    pub fn step(&mut self, hour: i32, inputs: Inputs) -> bool {
        let countries_changed = match inputs.countries {
            Input::Same => false,
            Input::Incomplete => self.country_keys.take().is_some(),
            Input::Read(keys) => {
                let changed = self
                    .country_keys
                    .as_ref()
                    .is_none_or(|held| !held.iter().eq(keys.iter()));
                if changed {
                    self.country_keys = Some(keys);
                }
                changed
            }
        };
        let links_changed = take(&mut self.links, inputs.links);
        let levels_changed = take(&mut self.levels, inputs.levels);
        self.relevel |= levels_changed;
        let relations_changed = match inputs.relations {
            Input::Same => false,
            Input::Incomplete => self.relations.take().is_some(),
            Input::Read(rows) => {
                let changed = self.relations.as_deref() != Some(&rows);
                if changed {
                    self.relations = Some(Arc::new(rows));
                }
                changed
            }
        };
        let adjacent = self.previous_hour.and_then(|last| last.checked_add(1)) == Some(hour);
        let expiry = self.next_expiry.is_some_and(|end| end <= hour);
        let changed = countries_changed
            || links_changed
            || relations_changed
            || levels_changed
            || expiry
            || !adjacent
            || !self.primed;
        self.primed = true;
        if !changed {
            self.advance_unchanged(hour, hour);
            return false;
        }
        let identities_changed =
            countries_changed || links_changed || self.country_keys.is_none();
        let reread = relations_changed || self.relations.is_none();
        let partial = !identities_changed && !reread && !expiry && adjacent;
        if !partial {
            self.seal();
        }
        let before = (self.incomplete, self.unleveled, self.unreadable);
        self.observe(hour, identities_changed, reread, partial);
        self.diagnostic_rate = (
            self.incomplete - before.0,
            self.unleveled - before.1,
            self.unreadable - before.2,
        );
        self.carried = self.diagnostic_rate.0 == 0;
        self.unobserved_here = self
            .unobserved
            .last()
            .is_some_and(|span| span.last_hour == hour);
        if reread || expiry {
            self.next_expiry = self.relations.as_ref().and_then(|rows| {
                rows.iter()
                    .filter(|row| {
                        row.cancel == 0 && row.end_hour > row.start_hour && row.end_hour > hour
                    })
                    .map(|row| row.end_hour)
                    .min()
            });
        }
        true
    }

    /// The step of hours `first..=last` whose relations, levels, holders and continuity are
    /// the hour before's and at which no relation ends: each hour adds the diagnostic rates of
    /// the last observed hour (as that many `+=` would, overflow included), an unobserved hour
    /// stays unobserved, and the frontier advances. Open runs are sealed at the next change.
    fn advance_unchanged(&mut self, first: i32, last: i32) {
        let hours = u64::try_from(i64::from(last) - i64::from(first) + 1).unwrap_or(0);
        self.incomplete = repeated(self.incomplete, self.diagnostic_rate.0, hours);
        self.unleveled = repeated(self.unleveled, self.diagnostic_rate.1, hours);
        self.unreadable = repeated(self.unreadable, self.diagnostic_rate.2, hours);
        if self.unobserved_here {
            mark_unobserved(&mut self.unobserved, first, last);
        }
        self.previous_hour = Some(last);
        if self.previous_complete.is_some() {
            self.previous_complete = Some(last);
        }
    }

    fn seal(&mut self) {
        if !self.carried {
            return;
        }
        let Some(hour) = self.previous_hour else {
            return;
        };
        for &index in self.open.values() {
            let run = &mut self.runs[index];
            if run.last_hour < hour {
                run.last_hour = hour;
                if let Some(step) = run.steps.last_mut() {
                    step.last_hour = hour;
                }
            }
        }
    }

    fn unobserved_hour(&mut self, hour: i32) {
        self.incomplete += 1;
        mark_unobserved(&mut self.unobserved, hour, hour);
        self.continuous_since = None;
        self.previous_complete = None;
    }

    fn observe(
        &mut self,
        hour: i32,
        identities_changed: bool,
        relations_changed: bool,
        partial: bool,
    ) {
        let old_frontier = self.previous_hour;
        let old_levels = self.resolved_levels.clone();
        let old_complete = self.previous_complete.is_some();
        let first = *self.first_observed.get_or_insert(hour);
        if let Some(previous) = self.previous_hour
            && hour > previous + 1
        {
            mark_unobserved(&mut self.unobserved, previous + 1, hour - 1);
            self.continuous_since = None;
        }
        self.previous_hour = Some(hour);
        let updated = if identities_changed {
            self.tags
                .update_projected(self.country_keys.as_ref(), self.links.as_ref())
                .ok()
        } else {
            (self.country_keys.is_some() && self.links.is_some()).then_some(false)
        };
        let Some(rebuilt) = updated else {
            self.unobserved_hour(hour);
            return;
        };
        if rebuilt || identities_changed {
            self.identities_changed = true;
            // The levels are by tag: resolved again at the next hour holding relations.
            self.relevel = true;
            self.identities = self
                .country_keys
                .as_ref()
                .expect("known country operands")
                .iter()
                .map(|(id, slot, _)| (slot, id))
                .collect();
            self.identity_since
                .retain(|slot, (id, _)| self.identities.get(slot) == Some(id));
            for (&slot, &id) in &self.identities {
                self.identity_since.entry(slot).or_insert((id, hour));
            }
            self.named_since
                .retain(|raw, (tag, _)| self.tags.get(*raw) == Some(tag));
            for (raw, tag) in self.tags.iter() {
                self.named_since
                    .entry(*raw)
                    .or_insert_with(|| (tag.clone(), hour));
            }
        }
        if relations_changed || identities_changed {
            self.relations_by_tag.clear();
            if let Some(rows) = &self.relations {
                for (index, row) in rows.iter().enumerate() {
                    if let Some(tag) = self.tags.get(row.subject) {
                        self.relations_by_tag
                            .entry(tag.clone())
                            .or_default()
                            .push(index);
                    }
                }
            }
        }
        let Some(rows) = self.relations.clone() else {
            self.unobserved_hour(hour);
            return;
        };
        self.observed = true;
        let since = *self.continuous_since.get_or_insert(hour);
        // Each subject's current level this hour, by tag; none when the table is not
        // complete. A subject the rows give two levels has no known level.
        if self.relevel {
            self.relevel = false;
            self.resolved_levels = self.levels.as_ref().map(|autonomy| {
                let mut levels: HashMap<String, Option<Vec<u8>>> = HashMap::new();
                for (subject, state) in autonomy {
                    let Some(tag) = self.tags.get(*subject) else {
                        continue;
                    };
                    let level = levels
                        .entry(tag.clone())
                        .or_insert_with(|| Some(state.clone()));
                    if level.as_deref() != Some(state.as_slice()) {
                        self.conflicting.insert(tag.clone());
                        *level = None;
                    }
                }
                Arc::new(levels)
            });
        }
        let levels = self.resolved_levels.clone();
        let selected: Vec<usize> = if partial {
            let tags: BTreeSet<_> = old_levels
                .as_ref()
                .into_iter()
                .flat_map(|levels| levels.keys())
                .chain(levels.as_ref().into_iter().flat_map(|levels| levels.keys()))
                .collect();
            tags.into_iter()
                .filter(|tag| {
                    old_levels
                        .as_ref()
                        .and_then(|levels| levels.get(*tag))
                        .and_then(|value| value.as_deref())
                        != levels
                            .as_ref()
                            .and_then(|levels| levels.get(*tag))
                            .and_then(|value| value.as_deref())
                })
                .flat_map(|tag| {
                    self.relations_by_tag
                        .get(tag)
                        .into_iter()
                        .flatten()
                        .copied()
                })
                .collect()
        } else {
            (0..rows.len()).collect()
        };
        if partial {
            self.incomplete += self.diagnostic_rate.0;
            self.unleveled += self.diagnostic_rate.1;
            self.unreadable += self.diagnostic_rate.2;
        }
        let mut named_all = !partial || old_complete;
        for row in selected
            .into_iter()
            .map(|index| &rows[index])
            .filter(|row| row.active(hour))
        {
            if partial
                && let (Some(overlord), Some(subject)) =
                    (self.tags.get(row.overlord), self.tags.get(row.subject))
            {
                let previous = old_levels
                    .as_ref()
                    .and_then(|levels| levels.get(subject))
                    .and_then(|level| level.as_deref());
                let unreadable = previous
                    .is_some_and(|bytes| std::str::from_utf8(bytes).ok().is_none_or(str::is_empty));
                self.unleveled -= u32::from(previous.is_none() || unreadable);
                self.unreadable -= u32::from(unreadable);
                if self.carried
                    && let Some(frontier) = old_frontier
                    && let Some(&index) =
                        self.open
                            .get(&(overlord.clone(), subject.clone(), row.start_hour))
                {
                    let run = &mut self.runs[index];
                    run.last_hour = frontier;
                    if let Some(step) = run.steps.last_mut() {
                        step.last_hour = frontier;
                    }
                }
            }
            let (Some(overlord), Some(subject)) =
                (self.tags.get(row.overlord), self.tags.get(row.subject))
            else {
                for raw in [row.overlord, row.subject] {
                    if self.tags.get(raw).is_none() {
                        self.unmapped.insert(raw);
                    }
                }
                named_all = false;
                continue;
            };
            // A level whose recorded id is not text (or is empty) is refused, and counted.
            let level = match levels
                .as_ref()
                .and_then(|levels| levels.get(subject).and_then(|level| level.as_deref()))
            {
                Some(bytes) => match std::str::from_utf8(bytes) {
                    Ok(token) if !token.is_empty() => Some(token.to_owned()),
                    _ => {
                        self.unreadable += 1;
                        None
                    }
                },
                None => None,
            };
            if level.is_none() {
                self.unleveled += 1;
            }
            let key = (overlord.clone(), subject.clone(), row.start_hour);
            let started = extend(&mut self.open, &mut self.runs, key.clone(), hour, || {
                SubjectRun {
                    overlord: overlord.clone(),
                    subject: subject.clone(),
                    start_hour: row.start_hour,
                    first_hour: hour,
                    last_hour: hour,
                    steps: Vec::new(),
                }
            });
            if started {
                let identity_key = [row.overlord, row.subject].map(|raw| {
                    self.links
                        .as_ref()
                        .and_then(|links| links.get(&raw))
                        .copied()
                        .and_then(|slot| self.identities.get(&slot).map(|id| (slot, *id)))
                });
                let identity_key = identity_key[0]
                    .zip(identity_key[1])
                    .map(|(overlord, subject)| (overlord, subject, row.start_hour));
                let new_identity =
                    identity_key.is_some_and(|identity| self.seen_starts.insert(identity));
                if new_identity && row.start_hour > first {
                    // Relations serialize start_date; names are exact only while both raw
                    // links keep their tags. A start at or before the first observed frame is
                    // an initial snapshot, even if first published later.
                    let date_named = [row.overlord, row.subject].into_iter().all(|raw| {
                        self.named_since
                            .get(&raw)
                            .is_some_and(|(_, from)| since.max(*from) <= row.start_hour)
                    }) && identity_key.is_some_and(|(overlord, subject, _)| {
                        [overlord, subject].into_iter().all(|(slot, id)| {
                            self.identity_since
                                .get(&slot)
                                .is_some_and(|(seen, from)| *seen == id && *from <= row.start_hour)
                        })
                    });
                    let exact = date_named && row.start_hour <= hour;
                    let witnessed = self
                        .previous_complete
                        .is_some_and(|previous: i32| previous.checked_add(1) == Some(hour))
                        && identity_key.is_some_and(|(overlord, subject, _)| {
                            [overlord, subject]
                                .into_iter()
                                .all(|(slot, id)| self.previous_identities.get(&slot) == Some(&id))
                        });
                    if exact || witnessed {
                        self.starts.push(SubjectStart {
                            subject: subject.clone(),
                            overlord: overlord.clone(),
                            hour: if exact { row.start_hour } else { hour },
                            read_hour: hour,
                            exact,
                        });
                    }
                }
            }
            let run = &mut self.runs[self.open[&key]];
            match run.steps.last_mut() {
                Some(step) if step.last_hour == hour => {}
                Some(step) if step.level == level && step.last_hour + 1 == hour && !started => {
                    step.last_hour = hour;
                }
                _ => {
                    run.steps.push(LevelStep {
                        first_hour: hour,
                        last_hour: hour,
                        level,
                    });
                }
            }
        }
        if !named_all {
            mark_unobserved(&mut self.unobserved, hour, hour);
        }
        self.previous_complete = named_all.then_some(hour);
        if self.identities_changed {
            self.previous_identities.clone_from(&self.identities);
            self.identities_changed = false;
        }
        if !partial {
            self.open
                .retain(|_, index| self.runs[*index].last_hour == hour);
        }
    }

    /// The relations going on from the hour last stepped, each run with the level it holds then:
    /// none after an hour whose tables were not complete.
    pub fn continuing(&self) -> impl Iterator<Item = &SubjectRun> + '_ {
        self.open
            .values()
            .filter(|_| self.carried)
            .map(|&index| &self.runs[index])
    }

    /// Whether `hour`, the hour last stepped, was unobserved.
    pub fn unobserved_at(&self, hour: i32) -> bool {
        self.unobserved
            .last()
            .is_some_and(|span| span.first_hour <= hour && hour <= span.last_hour)
    }

    /// The starts found since the last take, in the order they were found.
    pub fn take_starts(&mut self) -> Vec<SubjectStart> {
        std::mem::take(&mut self.starts)
    }

    /// Forgets the runs that ended and the unobserved spans before the last: the emit has
    /// published them, and nothing the scan does reads them again.
    pub fn prune(&mut self) {
        if self.unobserved.len() > 1 {
            self.unobserved.drain(..self.unobserved.len() - 1);
        }
        if self.runs.len() <= self.open.len() {
            return;
        }
        let mut runs = Vec::with_capacity(self.open.len());
        for index in self.open.values_mut() {
            runs.push(self.runs[*index].clone());
            *index = runs.len() - 1;
        }
        self.runs = runs;
    }

    /// What the history counted through the hour last stepped.
    pub fn counts(&self) -> super::Counts {
        super::Counts {
            unmapped: self.unmapped.len() as u64,
            incomplete: u64::from(self.incomplete),
            unleveled: u64::from(self.unleveled),
            unreadable: u64::from(self.unreadable),
            conflicting: self.conflicting.len() as u64,
            unreadable_blocks: 0,
            stopped: 0,
            observed: self.observed,
        }
    }
}
