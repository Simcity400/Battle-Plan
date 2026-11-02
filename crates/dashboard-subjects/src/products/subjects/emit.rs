//! Subjects' emit: the subject scan ([`super::scan`]) stepped through each projected hour, its
//! relations as runs on the family's series publication. Each relation (overlord, subject and
//! start hour) is a key of its subject's actor, holding the subject's level through every hour
//! the relation was observed; the family's keys hold the hours whose relations were not observed
//! and each relation start the event feed lists, as an event. The levels' ids and the scan's
//! counts are scalars.
//!
//! A relation's key goes on while the scan's run of it goes on; an hour not walked ends every
//! run and is unobserved.

use super::{
    Counts, SubjectStart, facts,
    scan::{Inputs, Scan},
};
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use std::{
    collections::{BTreeSet, HashMap},
    io,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// A subject's relations, by [`dimension`]: 0 where its level is unknown, else one more than
/// the level's id in [`LEVELS`].
pub const RELATION: u64 = 0;
/// The family's keys: the hours whose relations were not observed (1), and the starts.
pub const UNOBSERVED: u64 = 0;
pub const STARTS: u64 = 1;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// Family scalars: each level's id, by position; and the counts ([`counts_scalar`]).
pub const LEVELS: u64 = 0;
pub const COUNTS: u64 = 1;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// Event kinds.
pub mod kind {
    /// `[Unsigned subject actor, Unsigned overlord actor, Boolean exact]` on the STARTS key:
    /// dated at the start's hour, read at the hour that found it.
    pub const START: u64 = 0;
}

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved subjects emit is not valid")
}

/// A relation's dimension: its overlord's actor and its start hour.
pub fn dimension(overlord: u64, start_hour: i32) -> io::Result<u64> {
    if overlord >> 32 != 0 {
        return Err(dashboard_core::invalid("Subject overlords exceed 32 bits"));
    }
    Ok(overlord << 32 | u64::from(start_hour as u32))
}

/// The overlord actor and start hour of a relation's dimension.
pub fn relation_of(dimension: u64) -> (u64, i32) {
    (dimension >> 32, dimension as u32 as i32)
}

/// `counts` as the COUNTS scalar holds them.
pub fn counts_scalar(counts: &Counts) -> Scalar {
    Scalar::List(
        [
            counts.unmapped,
            counts.incomplete,
            counts.unleveled,
            counts.unreadable,
            counts.conflicting,
            u64::from(counts.observed),
        ]
        .into_iter()
        .map(Scalar::Unsigned)
        .collect(),
    )
}

/// The counts the COUNTS scalar holds; none before the first cut.
pub fn counts_of(scalar: Option<&Scalar>) -> io::Result<Counts> {
    let Some(scalar) = scalar else {
        return Ok(Counts::default());
    };
    let count = |at: usize| {
        scalar
            .list()
            .get(at)
            .and_then(Scalar::integer)
            .and_then(|count| u64::try_from(count).ok())
            .ok_or_else(|| dashboard_core::invalid("Prepared subject counts are not valid"))
    };
    Ok(Counts {
        unmapped: count(0)?,
        incomplete: count(1)?,
        unleveled: count(2)?,
        unreadable: count(3)?,
        conflicting: count(4)?,
        unreadable_blocks: 0,
        stopped: 0,
        observed: count(5)? != 0,
    })
}

/// The level ids the LEVELS scalar holds, by id.
pub fn levels_of(directory: &Directory) -> io::Result<Vec<String>> {
    directory
        .scalar(LEVELS)
        .map_or(&[][..], Scalar::list)
        .iter()
        .map(|level| {
            level
                .text()
                .and_then(|text| String::from_utf8(text.to_vec()).ok())
                .ok_or_else(|| dashboard_core::invalid("A prepared subject level is not valid"))
        })
        .collect()
}

/// The family's emit on one branch.
pub struct Emit {
    scan: Scan,
    actors: Actors,
    carry: Carry,
    /// Each relation's key, by overlord, subject and start hour.
    relations: HashMap<(String, String, i32), KeyHandle>,
    /// The relation keys holding a level now.
    valued: BTreeSet<KeyHandle>,
    /// UNOBSERVED and STARTS.
    family: Option<[KeyHandle; 2]>,
    /// Each level's id, and the ids by level.
    levels: Vec<String>,
    level_ids: HashMap<String, u64>,
    /// The levels and the counts changed since the last cut.
    levels_stale: bool,
    counts: Option<Counts>,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
    /// Taken up from a save: the scan holds no table until an hour reads every one.
    resumed: bool,
}

impl Emit {
    /// The emit of a branch whose first hour is `first`.
    pub fn new(first: i32) -> Self {
        Self {
            scan: Scan::starting(first),
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            relations: HashMap::new(),
            valued: BTreeSet::new(),
            family: None,
            levels: Vec::new(),
            level_ids: HashMap::new(),
            levels_stale: false,
            counts: None,
            last: None,
            first: None,
            resumed: false,
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
        for (key, measure) in keys.iter_mut().zip([UNOBSERVED, STARTS]) {
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

    /// The key of the relation of `overlord` over `subject` begun at `start_hour`.
    fn relation(
        &mut self,
        writer: &mut Writer<'_>,
        relation: (String, String, i32),
        hour: i32,
    ) -> io::Result<KeyHandle> {
        if let Some(&key) = self.relations.get(&relation) {
            return Ok(key);
        }
        let subject = self.actors.actor(writer, relation.1.as_bytes(), hour)?;
        let overlord = self.actors.actor(writer, relation.0.as_bytes(), hour)?;
        let key = self.key(
            writer,
            Place {
                actor: subject,
                measure: RELATION,
                dimension: Some(dimension(overlord, relation.2)?),
            },
        )?;
        self.relations.insert(relation, key);
        Ok(key)
    }

    /// The value a relation holds at `level`.
    fn level(&mut self, level: Option<&str>) -> i128 {
        let Some(level) = level else {
            return 0;
        };
        let id = match self.level_ids.get(level) {
            Some(&id) => id,
            None => {
                let id = self.levels.len() as u64;
                self.levels.push(level.to_owned());
                self.level_ids.insert(level.to_owned(), id);
                self.levels_stale = true;
                id
            }
        };
        i128::from(id) + 1
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("subjects.emit", None);
        let at = hour.hour;
        if self.last.is_some_and(|last| at <= last) {
            return Err(dashboard_core::invalid(
                "A subjects hour does not follow the hour before",
            ));
        }
        let gap = self
            .last
            .and_then(|last| last.checked_add(1))
            .filter(|&next| next < at);
        if std::mem::take(&mut self.resumed)
            && !hour.inputs.as_deref().is_some_and(Inputs::whole)
        {
            return Err(dashboard_core::invalid(
                "A subjects emit taken up again reads every table at its first hour",
            ));
        }
        self.last = Some(at);
        self.first.get_or_insert(gap.unwrap_or(at));
        let [unobserved, starts] = self.family(writer)?;
        let observed = self
            .scan
            .step(at, hour.inputs.map_or_else(Inputs::default, |inputs| *inputs));
        if let Some(next) = gap {
            // An hour not walked ends every relation, and is unobserved.
            for key in std::mem::take(&mut self.valued) {
                self.carry.value(writer, key, next, None, false)?;
            }
            self.carry.value(writer, unobserved, next, Some(1), false)?;
        }
        if observed {
            self.relate(writer, at)?;
        }
        let missing = self.scan.unobserved_at(at);
        self.carry
            .value(writer, unobserved, at, missing.then_some(1), false)?;
        for start in self.scan.take_starts() {
            self.start(writer, starts, start)?;
        }
        if observed {
            self.scan.prune();
        }
        Ok(())
    }

    /// The relations going on from `at`, each at its level, and those that ended before it.
    fn relate(&mut self, writer: &mut Writer<'_>, at: i32) -> io::Result<()> {
        let continuing: Vec<_> = self
            .scan
            .continuing()
            .map(|run| {
                (
                    (run.overlord.clone(), run.subject.clone(), run.start_hour),
                    run.steps.last().and_then(|step| step.level.clone()),
                    run.first_hour == at,
                )
            })
            .collect();
        let mut valued = BTreeSet::new();
        for (relation, level, started) in continuing {
            let key = self.relation(writer, relation, at)?;
            let value = self.level(level.as_deref());
            self.carry.value(writer, key, at, Some(value), started)?;
            valued.insert(key);
        }
        for key in std::mem::replace(&mut self.valued, valued) {
            if !self.valued.contains(&key) {
                self.carry.value(writer, key, at, None, false)?;
            }
        }
        Ok(())
    }

    /// The start's event.
    fn start(
        &mut self,
        writer: &mut Writer<'_>,
        key: KeyHandle,
        start: SubjectStart,
    ) -> io::Result<()> {
        let subject = self
            .actors
            .actor(writer, start.subject.as_bytes(), start.read_hour)?;
        let overlord = self
            .actors
            .actor(writer, start.overlord.as_bytes(), start.read_hour)?;
        let fields = writer.cells([
            Cell::Unsigned(subject),
            Cell::Unsigned(overlord),
            Cell::Boolean(start.exact),
        ])?;
        let sequence = writer.sequence()?;
        writer.event(Event {
            key,
            native_hour: start.hour,
            read_hour: Some(start.read_hour),
            sequence,
            kind: kind::START,
            fields,
        })
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
        if self.levels_stale {
            let levels = Scalar::List(
                self.levels
                    .iter()
                    .map(|level| Scalar::Text(level.as_bytes().into()))
                    .collect(),
            );
            let cell = family::cell(writer, &levels)?;
            writer.catalogue(LEVELS, cell)?;
            self.levels_stale = false;
        }
        let counts = self.scan.counts();
        if self.counts != Some(counts) {
            let cell = family::cell(writer, &counts_scalar(&counts))?;
            writer.catalogue(COUNTS, cell)?;
            self.counts = Some(counts);
        }
        Ok(())
    }

    /// What the emit goes on with after a cut: its scan, the relation keys holding a level and
    /// its last hour, then its carry.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let valued: Vec<KeyHandle> = self.valued.iter().copied().collect();
        let json = serde_json::to_vec(&(&self.scan, valued, self.last))?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let (scan, valued, last): (Scan, Vec<KeyHandle>, Option<i32>) =
            serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        let mut emit = Self::new(0);
        emit.scan = scan;
        emit.valued = valued.into_iter().collect();
        emit.last = last;
        emit.carry = Carry::load(&bytes[at..])?;
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let mut family = [0; 2];
        for (key, measure) in family.iter_mut().zip([UNOBSERVED, STARTS]) {
            *key = directory.key(FAMILY, measure, None).ok_or_else(invalid)?;
        }
        emit.family = Some(family);
        let name = |actor: u64| {
            directory
                .actors
                .get(actor as usize)
                .and_then(|actor| String::from_utf8(actor.name.to_vec()).ok())
                .ok_or_else(invalid)
        };
        for subject in 0..directory.actors.len() as u64 {
            for &(dimension, key) in directory.dimensions(subject, RELATION) {
                let (overlord, start_hour) = relation_of(dimension);
                emit.relations
                    .insert((name(overlord)?, name(subject)?, start_hour), key);
            }
        }
        emit.levels = levels_of(directory)?;
        emit.level_ids = (0..)
            .zip(&emit.levels)
            .map(|(id, level)| (level.clone(), id))
            .collect();
        emit.counts = Some(counts_of(directory.scalar(COUNTS))?);
        emit.resumed = true;
        Ok(emit)
    }
}
