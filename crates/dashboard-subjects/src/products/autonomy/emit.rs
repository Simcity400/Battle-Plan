//! Subject autonomy's emit: each subject tag's score, move-down and move-up scores, relation
//! ratio and its place between the two scores as runs on the family's series publication, and
//! its band (overlord, level, whether a level lies below or above, whether a relation names a
//! ratio) as runs of the band's ordinal with each band as an event where it starts; the hours
//! recorded and those naming a country the recording does not; and the levels in the order first
//! seen with their lowest move-down score, the subjects each overlord ever held and the
//! subject-hours with more than one overlord.
//!
//! A subject's runs go on while it keeps its holder and is observed every hour. A holder change,
//! an hour it is not observed, an hour not recorded and an hour not walked end them; a band
//! starts anew wherever its fields change or its runs end, and its score's run with it.

use super::facts::{self, Sample};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A subject's measures: its score, move-down and move-up scores, ratio, its place between the
/// two scores (billionths) and its band's ordinal, each band a [`kind::BAND`] event.
pub const SCORE: u64 = 0;
pub const LOWER: u64 = 1;
pub const UPPER: u64 = 2;
pub const RATIO: u64 = 3;
pub const RELATIVE: u64 = 4;
pub const BAND: u64 = 5;
/// The family's keys: the hours recorded, and those naming a country the recording does not.
pub const RECORDED: u64 = 0;
pub const UNIDENTIFIED: u64 = 1;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// Family scalars: each level's id and lowest move-down score in the order first seen, the
/// subject-hours with more than one overlord, and each overlord's subjects.
pub const LEVELS: u64 = 0;
pub const SHARED: u64 = 1;
pub const OVERLORDS: u64 = 2;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;
const MEASURES: [u64; 6] = [SCORE, LOWER, UPPER, RATIO, RELATIVE, BAND];

/// Event kinds.
pub mod kind {
    /// `[Integer previous | Null, Boolean starts, Text overlord | Null, Unsigned level,
    /// Boolean lowest, Boolean highest, Boolean ratio]` on a subject's BAND key: the last hour
    /// the subject was observed before, whether its runs start anew, its overlord, its level's
    /// place among [`super::LEVELS`], and whether no level lies below or above and a relation
    /// names its ratio.
    pub const BAND: u64 = 0;
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved autonomy emit is not valid")
}

/// A subject's five numeric values, as the reply reads them.
pub fn values(sample: &Sample) -> [Option<i128>; 5] {
    let (score, lower, upper) = (
        i128::from(sample.score),
        i128::from(sample.lower),
        i128::from(sample.upper),
    );
    [
        Some(score),
        Some(lower),
        Some(upper),
        sample.ratio.map(i128::from),
        (upper != lower).then(|| (score - lower) * 1_000_000_000 / (upper - lower)),
    ]
}

/// What a band holds besides its hours.
fn same_band(left: &Sample, right: &Sample) -> bool {
    left.overlord == right.overlord
        && left.level == right.level
        && left.lowest == right.lowest
        && left.highest == right.highest
        && left.ratio.is_some() == right.ratio.is_some()
}

/// What a subject holds now, and its keys.
#[derive(Clone, Default)]
struct Subject {
    keys: [KeyHandle; 6],
    /// The projection's sample of it, while one names it.
    sample: Option<Arc<Sample>>,
    /// The sample its runs hold, and whether they held it the hour before.
    held: Option<Arc<Sample>>,
    active: bool,
    /// The last hour it was observed, while it is not.
    seen: Option<i32>,
    /// Its band's ordinal.
    band: u64,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    labels: Vec<Arc<str>>,
    subjects: Vec<Subject>,
    /// RECORDED and UNIDENTIFIED.
    family: Option<[KeyHandle; 2]>,
    levels: Vec<(Arc<[u8]>, i64)>,
    level_of: HashMap<Arc<[u8]>, u64>,
    overlords: BTreeMap<Arc<str>, BTreeSet<Arc<str>>>,
    /// Whether every table read is complete, and the hour's flags.
    recorded: bool,
    unidentified: bool,
    shared: u32,
    shared_hours: u64,
    /// Every named subject is observed at the next recorded hour: none held runs before it.
    restart: bool,
    /// The scalars changed since the last cut.
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
            labels: Vec::new(),
            subjects: Vec::new(),
            family: None,
            levels: Vec::new(),
            level_of: HashMap::new(),
            overlords: BTreeMap::new(),
            recorded: false,
            unidentified: false,
            shared: 0,
            shared_hours: 0,
            restart: true,
            stale: true,
            last: None,
            first: None,
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))
    }

    fn family(&mut self, writer: &mut Writer<'_>) -> io::Result<[KeyHandle; 2]> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let mut keys = [0; 2];
        for (key, measure) in keys.iter_mut().zip([RECORDED, UNIDENTIFIED]) {
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

    /// The subject actor named `tag`, with its keys, made at `hour` when it is new.
    fn subject(&mut self, writer: &mut Writer<'_>, tag: &Arc<str>, hour: i32) -> io::Result<u64> {
        if let Some(actor) = self.actors.find(tag.as_bytes()) {
            return Ok(actor);
        }
        let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
        let index = actor as usize;
        if index >= self.subjects.len() {
            self.subjects.resize(index + 1, Subject::default());
            self.labels.resize(index + 1, Arc::from(""));
        }
        self.labels[index] = Arc::clone(tag);
        let mut keys = [0; 6];
        for (key, measure) in keys.iter_mut().zip(MEASURES) {
            *key = self.key(
                writer,
                Place {
                    actor,
                    measure,
                    dimension: None,
                },
            )?;
        }
        self.subjects[index].keys = keys;
        Ok(actor)
    }

    /// The place of `level` among the levels, in the order first seen.
    fn level(&mut self, level: &Arc<[u8]>, lower: i64) -> u64 {
        if let Some(&at) = self.level_of.get(level) {
            let lowest = &mut self.levels[at as usize].1;
            if lower < *lowest {
                *lowest = lower;
                self.stale = true;
            }
            return at;
        }
        let at = self.levels.len() as u64;
        self.levels.push((Arc::clone(level), lower));
        self.level_of.insert(Arc::clone(level), at);
        self.stale = true;
        at
    }

    /// What one row of `actor`'s observed counts: its level, move-down score and overlord; the
    /// level's place.
    fn contribute(&mut self, actor: u64, sample: &Sample) -> u64 {
        let level = self.level(&sample.level, sample.lower);
        if let Some(overlord) = &sample.overlord
            && self
                .overlords
                .entry(Arc::clone(overlord))
                .or_default()
                .insert(Arc::clone(&self.labels[actor as usize]))
        {
            self.stale = true;
        }
        level
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("autonomy.emit", None);
        let at = hour.hour;
        let adjacent = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => true,
            Some(last) if at <= last => {
                return Err(dashboard_core::invalid(
                    "An autonomy hour does not follow the hour before",
                ));
            }
            // An hour not walked ends every run.
            Some(last) => {
                self.close(writer, last)?;
                false
            }
            None => false,
        };
        self.last = Some(at);
        self.first.get_or_insert(at);
        let [recorded, unidentified] = self.family(writer)?;
        let mut observed: Vec<u64> = Vec::new();
        if let Some(facts) = hour.facts {
            let facts::Facts {
                recorded: complete,
                whole,
                changes,
                unidentified: unnamed,
                shared,
            } = *facts;
            if let Some(complete) = complete {
                self.recorded = complete;
            }
            if let Some(unnamed) = unnamed {
                self.unidentified = unnamed;
            }
            if let Some(shared) = shared {
                self.shared = shared;
            }
            let mut named = BTreeSet::new();
            let mut rows: Vec<(u32, u64, Arc<Sample>)> = Vec::new();
            for change in changes {
                let actor = match &change.sample {
                    Some(_) => self.subject(writer, &change.tag, at)?,
                    None => match self.actors.find(change.tag.as_bytes()) {
                        Some(actor) => actor,
                        None => continue,
                    },
                };
                named.insert(actor);
                rows.extend(
                    change
                        .rows
                        .into_iter()
                        .map(|(subject, sample)| (subject, actor, sample)),
                );
                self.subjects[actor as usize].sample = change.sample;
                observed.push(actor);
            }
            // Every subject the changes of a whole hour do not name holds no sample.
            if whole {
                for (actor, subject) in self.subjects.iter_mut().enumerate() {
                    if subject.sample.is_some() && !named.contains(&(actor as u64)) {
                        subject.sample = None;
                        observed.push(actor as u64);
                    }
                }
            }
            // Every row counts its level, move-down score and overlord, in id order across tags.
            rows.sort_unstable_by_key(|&(subject, ..)| subject);
            for (_, actor, sample) in rows {
                self.contribute(actor, &sample);
            }
        }
        if !self.recorded {
            // An hour without every table is not recorded: every run ends before it.
            for actor in 0..self.subjects.len() as u64 {
                self.deactivate(writer, actor, at)?;
            }
            self.restart = true;
            self.carry.value(writer, recorded, at, None, false)?;
            return self.carry.value(writer, unidentified, at, None, false).map(drop);
        }
        self.carry.value(writer, recorded, at, Some(1), false)?;
        let flag = self.unidentified.then_some(1);
        self.carry.value(writer, unidentified, at, flag, false)?;
        if self.shared > 0 {
            self.shared_hours += u64::from(self.shared);
            self.stale = true;
        }
        if std::mem::take(&mut self.restart) {
            let held = observed.iter().copied().collect::<BTreeSet<_>>();
            observed.extend(
                (0..self.subjects.len() as u64).filter(|actor| {
                    !held.contains(actor) && self.subjects[*actor as usize].sample.is_some()
                }),
            );
        }
        for actor in observed {
            if self.subjects[actor as usize].sample.is_some() {
                self.observe(writer, actor, at, adjacent)?;
            } else {
                self.deactivate(writer, actor, at)?;
            }
        }
        Ok(())
    }

    /// `actor`'s sample observed at `at`: its band starts anew where its fields changed or its
    /// runs do not go on from the hour before, and each value changed starts a run.
    fn observe(&mut self, writer: &mut Writer<'_>, actor: u64, at: i32, adjacent: bool) -> io::Result<()> {
        let index = actor as usize;
        let sample = self.subjects[index]
            .sample
            .clone()
            .expect("an observed subject's sample");
        let level = self.contribute(actor, &sample);
        let subject = &self.subjects[index];
        if subject.active
            && adjacent
            && subject
                .held
                .as_ref()
                .is_some_and(|held| Arc::ptr_eq(held, &sample) || **held == *sample)
        {
            return Ok(());
        }
        let continues = subject.active
            && adjacent
            && subject
                .held
                .as_ref()
                .is_some_and(|held| held.holder == sample.holder);
        let changed = !continues
            || subject
                .held
                .as_ref()
                .is_none_or(|held| !same_band(held, &sample));
        let keys = subject.keys;
        if changed {
            let previous = if subject.active {
                at.checked_sub(1)
            } else {
                subject.seen
            };
            let band = subject.band + 1;
            self.subjects[index].band = band;
            self.carry
                .value(writer, keys[5], at, Some(i128::from(band)), !continues)?;
            let overlord = match sample.overlord.as_deref() {
                Some("") => Cell::Text(dashboard_series::Blob::EMPTY),
                Some(tag) => Cell::Text(writer.text(tag)?),
                None => Cell::Null,
            };
            let fields = writer.cells([
                previous.map_or(Cell::Null, |hour| Cell::Integer(i128::from(hour))),
                Cell::Boolean(!continues),
                overlord,
                Cell::Unsigned(level),
                Cell::Boolean(sample.lowest),
                Cell::Boolean(sample.highest),
                Cell::Boolean(sample.ratio.is_some()),
            ])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key: keys[5],
                native_hour: at,
                read_hour: None,
                sequence,
                kind: kind::BAND,
                fields,
            })?;
        }
        for (field, value) in values(&sample).into_iter().enumerate() {
            let key = keys[field];
            // A score starts a run with its band; a value after an hour of none breaks.
            let starts = !continues
                || (field == SCORE as usize && changed)
                || self.carry.held(key).is_none();
            self.carry.value(writer, key, at, value, starts)?;
        }
        let subject = &mut self.subjects[index];
        subject.held = Some(sample);
        subject.active = true;
        subject.seen = None;
        Ok(())
    }

    /// Ends `actor`'s runs before `at`, the hour after its last observation.
    fn deactivate(&mut self, writer: &mut Writer<'_>, actor: u64, at: i32) -> io::Result<()> {
        let subject = &mut self.subjects[actor as usize];
        if !subject.active {
            return Ok(());
        }
        subject.active = false;
        subject.seen = at.checked_sub(1);
        let keys = subject.keys;
        for key in keys {
            self.carry.value(writer, key, at, None, false)?;
        }
        Ok(())
    }

    /// Ends every run after `last`, the hour before one not walked.
    fn close(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.close(writer, last)?;
        for subject in &mut self.subjects {
            if std::mem::take(&mut subject.active) {
                subject.seen = Some(last);
            }
        }
        self.restart = true;
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, the hours since the last cut as known, and the scalars that changed.
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
            let text = |bytes: &[u8]| Scalar::Text(bytes.into());
            let scalars = [
                (
                    LEVELS,
                    Scalar::List(
                        self.levels
                            .iter()
                            .map(|(id, lowest)| {
                                Scalar::List(vec![text(id), Scalar::Integer(i128::from(*lowest))])
                            })
                            .collect(),
                    ),
                ),
                (SHARED, Scalar::Unsigned(self.shared_hours)),
                (
                    OVERLORDS,
                    Scalar::List(
                        self.overlords
                            .iter()
                            .map(|(overlord, subjects)| {
                                Scalar::List(vec![
                                    text(overlord.as_bytes()),
                                    Scalar::List(
                                        subjects.iter().map(|tag| text(tag.as_bytes())).collect(),
                                    ),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ];
            for (id, scalar) in scalars {
                let cell = family::cell(writer, &scalar)?;
                writer.catalogue(id, cell)?;
            }
            self.stale = false;
        }
        Ok(())
    }

    /// What the emit goes on with after a cut. A restart goes on at a segment's first hour,
    /// whose projection names every sample anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            last: self.last,
            recorded: self.recorded,
            unidentified: self.unidentified,
            shared: self.shared,
            shared_hours: self.shared_hours,
            restart: self.restart,
            levels: self
                .levels
                .iter()
                .map(|(id, lowest)| (id.to_vec(), *lowest))
                .collect(),
            overlords: self
                .overlords
                .iter()
                .map(|(overlord, subjects)| {
                    (
                        overlord.to_string(),
                        subjects.iter().map(|tag| tag.to_string()).collect(),
                    )
                })
                .collect(),
            subjects: self
                .subjects
                .iter()
                .enumerate()
                .filter(|(_, subject)| subject.band != 0 || subject.sample.is_some())
                .map(|(actor, subject)| SavedSubject {
                    actor: actor as u64,
                    sample: subject.sample.as_deref().cloned(),
                    held: subject.held.as_deref().cloned(),
                    active: subject.active,
                    seen: subject.seen,
                    band: subject.band,
                })
                .collect(),
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
        emit.labels = directory
            .actors
            .iter()
            .map(|actor| Arc::from(&*String::from_utf8_lossy(&actor.name)))
            .collect();
        emit.subjects = vec![Subject::default(); directory.actors.len()];
        for (actor, subject) in emit.subjects.iter_mut().enumerate() {
            for (key, measure) in subject.keys.iter_mut().zip(MEASURES) {
                *key = directory
                    .key(actor as u64, measure, None)
                    .ok_or_else(invalid)?;
            }
        }
        let mut family = [0; 2];
        for (key, measure) in family.iter_mut().zip([RECORDED, UNIDENTIFIED]) {
            *key = directory.key(FAMILY, measure, None).ok_or_else(invalid)?;
        }
        emit.family = Some(family);
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.recorded = saved.recorded;
        emit.unidentified = saved.unidentified;
        emit.shared = saved.shared;
        emit.shared_hours = saved.shared_hours;
        emit.restart = saved.restart;
        for (id, lowest) in saved.levels {
            let id: Arc<[u8]> = Arc::from(id);
            emit.level_of.insert(Arc::clone(&id), emit.levels.len() as u64);
            emit.levels.push((id, lowest));
        }
        for (overlord, subjects) in saved.overlords {
            emit.overlords.insert(
                Arc::from(overlord),
                subjects.into_iter().map(Arc::from).collect(),
            );
        }
        for saved in saved.subjects {
            let subject = emit
                .subjects
                .get_mut(saved.actor as usize)
                .ok_or_else(invalid)?;
            subject.sample = saved.sample.map(Arc::new);
            subject.held = saved.held.map(Arc::new);
            subject.active = saved.active;
            subject.seen = saved.seen;
            subject.band = saved.band;
        }
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct SavedSubject {
    actor: u64,
    sample: Option<Sample>,
    held: Option<Sample>,
    active: bool,
    seen: Option<i32>,
    band: u64,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    recorded: bool,
    unidentified: bool,
    shared: u32,
    shared_hours: u64,
    restart: bool,
    levels: Vec<(Vec<u8>, i64)>,
    overlords: Vec<(String, Vec<String>)>,
    subjects: Vec<SavedSubject>,
}
