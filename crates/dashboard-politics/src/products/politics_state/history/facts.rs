//! The political state's facts, projected hour by hour from a columnar walk
//! ([`replay::segments`]): each part read ([`super::observe`]) at an hour that may change what
//! it reads ([`Part::reads`]), and handed on as what changed since the segment last handed it
//! on, whole at the segment's first hour and after a gap; with the holders' tags where they
//! changed and the hour's refusals where they changed. What came before the segment is the
//! ordered emit's ([`super::emit`]).

use super::{
    Field, Part,
    observe::{self, Observation, Val},
};
use crate::products::politics_state::Tags;
use dashboard_world::{Hold, Needs, World};
use replay::{
    columns,
    segments::{self, Operator, Sink},
};
use replay_data::rows::TableId;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = super::TABLES;

/// The fields of `table` named `names`, as change-mask bits.
fn mask(table: TableId, names: &[&str]) -> io::Result<u64> {
    names.iter().try_fold(0, |mask, name| {
        Ok(mask | 1_u64 << columns::field(table, name)?)
    })
}

/// How the walk's frame holds each table a part reads: by the fields the parts read.
fn holds() -> Vec<(TableId, Hold)> {
    let mut fields = BTreeMap::<TableId, u64>::new();
    for part in Part::ALL {
        for read in part.reads() {
            *fields.entry(read.table).or_default() |= read.fields;
        }
    }
    fields
        .into_iter()
        .map(|(table, fields)| (table, Hold::Fields(fields).of(table)))
        .collect()
}

/// What a projection reads of its walk's world: the frame of the parts' tables.
pub fn needs() -> Needs {
    Needs {
        frame: holds(),
        ..Needs::default()
    }
}

/// A part's values handed on: every value it holds, or what changed since the segment last
/// handed it on, none ending a value.
#[derive(replay_spool::Spooled)]
pub struct Sent {
    pub part: Part,
    pub whole: bool,
    pub values: Vec<((u32, Field), Option<Val>)>,
}

/// What the ordered emit reads of an hour that changes something.
#[derive(Default)]
#[derive(replay_spool::Spooled)]
pub struct Facts {
    /// Each holder's tag where it changed, every holder's at the segment's first hour.
    pub tags: Vec<(u32, Arc<str>)>,
    pub parts: Vec<Sent>,
    /// Whether each holder owns a state, where the hour says, where it changed; none while the
    /// civil-war lists are not read.
    pub existence: Option<Option<Arc<HashMap<u32, bool>>>>,
    /// The refusals holding from this hour, each with its count, where they changed.
    pub refusals: Option<Vec<(String, u64)>>,
}

/// One hour of a segment.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// None when the hour changes nothing.
    pub facts: Option<Box<Facts>>,
}

fn field_heap(field: &Field) -> usize {
    match field {
        Field::BalanceValue(balance)
        | Field::BalanceRange(balance)
        | Field::BalanceTrending(balance)
        | Field::BalanceRecord(balance) => balance.capacity(),
        _ => 0,
    }
}

/// `reason` without the hour its read named, which a refusal holding over many hours would
/// otherwise split at every read and every segment's first hour.
fn stable(reason: String, hour: i32) -> String {
    reason
        .replace(&format!(" at hour {hour}"), "")
        .replace(&format!("hour {hour} "), "")
}

/// Of a part: the tables and fields a change to which may change it.
struct Watch {
    reads: Vec<(TableId, u64)>,
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    watches: [Watch; 4],
    asks: Vec<(TableId, Hold)>,
    /// The identities' fields, which every part reads.
    identity: [(TableId, u64); 2],
    /// SliceReads' sample hour.
    sample: u64,
    /// Nothing is held: the next hour is read whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
    /// Each part's values as last handed on in the segment.
    held: [HashMap<(u32, Field), Val>; 4],
    /// Each part's refusals at its last read, and the identities'.
    refusals: [Vec<String>; 4],
    unnamed: Option<String>,
    sent: Option<BTreeMap<String, u64>>,
    tags: HashMap<u32, Arc<str>>,
    existence: Option<Arc<HashMap<u32, bool>>>,
    /// The raw countries in exile at the last read, whose read hours the exiles read.
    exiled: HashSet<u32>,
    /// The hours a war relation stops being ongoing, as of the civil wars' last read.
    ends: BTreeSet<i32>,
    #[cfg(test)]
    pub(crate) observed: Vec<(i32, Part)>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walk's tables, as at a segment's first hour.
const SAVED: &[u8] = b"politics state projector: cold";

impl Projector {
    pub fn new() -> io::Result<Self> {
        let identity = [
            (TableId::Countries, mask(TableId::Countries, &["canonical_slot", "tag"])?),
            (TableId::CountryLinks, mask(TableId::CountryLinks, &["canonical_slot"])?),
        ];
        let watches = Part::ALL.map(|part| Watch {
            reads: part
                .reads()
                .iter()
                .filter(|read| {
                    read.table != TableId::SliceReads
                        && !identity.iter().any(|&(table, fields)| {
                            table == read.table && read.fields & !fields == 0
                        })
                })
                .map(|read| (read.table, read.fields))
                .collect(),
        });
        Ok(Self {
            watches,
            asks: holds(),
            identity,
            sample: mask(TableId::SliceReads, &["sample_hour"])?,
            cold: true,
            last: None,
            held: Default::default(),
            refusals: Default::default(),
            unnamed: None,
            sent: None,
            tags: HashMap::new(),
            existence: None,
            exiled: HashSet::new(),
            ends: BTreeSet::new(),
            #[cfg(test)]
            observed: Vec::new(),
        })
    }

    /// Whether the hour changed the read hour of an exile's stability inputs.
    fn exile_read(&self, hour: &segments::Hour<'_, World>) -> io::Result<bool> {
        let table = TableId::SliceReads;
        if !hour.world.touched(table, self.sample) {
            return Ok(false);
        }
        if hour.world.restated(table) {
            return Ok(true);
        }
        let changes = columns::changes(hour.state, table)?;
        if changes.truncated {
            return Ok(true);
        }
        let sliced = TableId::CountryStabilityInputs.index() as u128;
        let exiled = |key: u128| key >> 32 == sliced && self.exiled.contains(&(key as u32));
        if changes.deleted_keys.iter().any(|&key| exiled(key)) {
            return Ok(true);
        }
        let held = columns::table(hour.state, table)?;
        let written = changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            .filter(|&(_, &mask)| mask & self.sample != 0)
            .map(|(&slot, _)| slot)
            .chain(changes.inserted.iter().copied());
        for slot in written {
            if held.key(slot).is_some_and(exiled) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The parts the hour may change.
    fn dirty(&self, hour: &segments::Hour<'_, World>, whole: bool) -> io::Result<([bool; 4], bool)> {
        let world = hour.world;
        let identity = whole
            || self
                .identity
                .iter()
                .any(|&(table, fields)| world.touched(table, fields));
        if identity {
            return Ok(([true; 4], true));
        }
        let mut dirty = [false; 4];
        for (part, watch) in Part::ALL.into_iter().zip(&self.watches) {
            dirty[part as usize] = watch
                .reads
                .iter()
                .any(|&(table, fields)| world.touched(table, fields));
        }
        if !dirty[Part::Exiles as usize] {
            dirty[Part::Exiles as usize] = self.exile_read(hour)?;
        }
        if !dirty[Part::CivilWars as usize] {
            dirty[Part::CivilWars as usize] = self.ends.contains(&hour.hour);
        }
        Ok((dirty, false))
    }

    /// `observation` of `part` handed on: whole, or as it differs from the last handed on.
    fn send(
        &mut self,
        part: Part,
        observation: Observation,
        whole: bool,
        hour: i32,
    ) -> Option<Sent> {
        let held = &mut self.held[part as usize];
        let now = observation.values;
        let mut values = Vec::new();
        if whole {
            values.extend(now.iter().map(|(key, value)| (key.clone(), Some(value.clone()))));
        } else {
            for (key, value) in &now {
                if held.get(key) != Some(value) {
                    values.push((key.clone(), Some(value.clone())));
                }
            }
            for key in held.keys() {
                if !now.contains_key(key) {
                    values.push((key.clone(), None));
                }
            }
        }
        *held = now;
        self.refusals[part as usize] = observation
            .refusals
            .into_iter()
            .map(|reason| stable(reason, hour))
            .collect();
        (whole || !values.is_empty()).then_some(Sent {
            part,
            whole,
            values,
        })
    }

    /// The refusals holding now, where they differ from the last handed on.
    fn refused(&mut self, whole: bool) -> Option<Vec<(String, u64)>> {
        let mut now = BTreeMap::<String, u64>::new();
        for reason in self.refusals.iter().flatten().chain(&self.unnamed) {
            *now.entry(reason.clone()).or_default() += 1;
        }
        if !whole && self.sent.as_ref() == Some(&now) {
            return None;
        }
        let refusals = now.iter().map(|(reason, &count)| (reason.clone(), count)).collect();
        self.sent = Some(now);
        Some(refusals)
    }

    fn heap(&self) -> usize {
        let entry = size_of::<((u32, Field), Val)>() + 1;
        self.held
            .iter()
            .map(|held| {
                held.capacity() * entry
                    + held
                        .iter()
                        .map(|((_, field), value)| field_heap(field) + value.heap())
                        .sum::<usize>()
            })
            .sum::<usize>()
            + self
                .refusals
                .iter()
                .flatten()
                .map(|reason| reason.capacity() + size_of::<String>())
                .sum::<usize>()
            + self.sent.as_ref().map_or(0, |sent| {
                sent.keys().map(|reason| reason.capacity() + 64).sum::<usize>()
            })
            + self.tags.capacity() * (size_of::<(u32, Arc<str>)>() + 1)
            + self.tags.values().map(|tag| tag.len()).sum::<usize>()
            + self
                .existence
                .as_ref()
                .map_or(0, |existence| existence.capacity() * (size_of::<(u32, bool)>() + 1))
            + self.exiled.capacity() * 5
            + self.ends.len() * 32
    }
}

// Every segment starts cold: its first hour reads every part whole; what came before is the
// emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("politics_state.columnar", Some("politics_state"));
        let at = hour.hour;
        let whole = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.cold = false;
        self.last = Some(at);
        if whole {
            self.held = Default::default();
            self.refusals = Default::default();
            self.unnamed = None;
            self.sent = None;
            self.tags.clear();
            self.existence = None;
        }
        let (dirty, identity) = self.dirty(hour, whole)?;
        if !dirty.contains(&true) {
            return out.push(Hour {
                branch: hour.branch,
                hour: at,
                facts: None,
            });
        }
        let frame = hour.world.frame_held(hour.state, &self.asks)?;
        let tags = Tags::new(&frame);
        let mut facts = Facts::default();
        self.unnamed = tags
            .as_ref()
            .err()
            .map(|error| stable(format!("country identities unavailable: {error}"), at));
        if identity && let Ok(tags) = &tags {
            for slot in tags.holders() {
                let Some(tag) = tags.slot_tag(slot) else {
                    continue;
                };
                if self.tags.get(&slot).is_none_or(|held| **held != *tag) {
                    let tag: Arc<str> = Arc::from(tag);
                    self.tags.insert(slot, Arc::clone(&tag));
                    facts.tags.push((slot, tag));
                }
            }
        }
        let named = tags.as_ref().ok();
        for part in Part::ALL {
            if !dirty[part as usize] {
                continue;
            }
            #[cfg(test)]
            self.observed.push((at, part));
            let mut observation = observe::observe(&frame, named, part);
            match part {
                Part::Exiles => {
                    self.exiled = observe::exile_countries(&frame).into_iter().collect();
                }
                Part::CivilWars => {
                    self.ends = observe::relation_ends(&frame);
                    let existence = observation.existence.take().map(Arc::new);
                    if whole || existence != self.existence {
                        facts.existence = Some(existence.clone());
                    }
                    self.existence = existence;
                }
                _ => {}
            }
            if let Some(sent) = self.send(part, observation, whole, at) {
                facts.parts.push(sent);
            }
        }
        facts.refusals = self.refused(whole);
        let quiet = facts.tags.is_empty()
            && facts.parts.is_empty()
            && facts.existence.is_none()
            && facts.refusals.is_none();
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            facts: (!quiet).then(|| Box::new(facts)),
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(self.heap())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved politics state projector",
            ));
        }
        *self = Self::new()?;
        Ok(())
    }
}
