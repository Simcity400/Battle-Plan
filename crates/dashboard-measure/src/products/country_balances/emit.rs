//! Country balances' emit: each tag's political and command power as runs on the family's
//! series publication, each run's holder (slot and country id, and the first hour of the segment
//! it holds) as its evidence, and each net drop between consecutive hours as an event of its key.
//!
//! A balance's run goes on while its tag's holder stays and its value stays known; a new holder,
//! a value after an unknown one, a tag held again and an unread hour start a segment. A drop is
//! counted only within a segment. A tag no resolved country holds ends its runs.

use super::facts;
use dashboard_series::{
    Cell, Event, RunMode, Writer,
    carry::{Carry, Read},
    codec::Decode,
    family::{Actors, Directory, Place},
};
use std::{collections::HashSet, io, sync::Arc};

/// The family's balance keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// Political power, then command power.
pub const MEASURES: [u64; 2] = [0, 1];
/// Both recorded balances are fixed5.
pub const DECIMALS: u8 = dashboard_base::products::country_measure::MEASURE_DECIMALS;
/// The event kind of a net drop: its balance before and after, as fields.
pub const DROP: u64 = 1;

/// The measure names a reply asks by.
pub fn measure(name: &str) -> Option<u64> {
    Some(match name {
        "politicalPower" => MEASURES[0],
        "commandPower" => MEASURES[1],
        _ => return None,
    })
}

/// What a tag's last offer held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Actor {
    keys: [u64; 2],
    holder: Option<(u32, u32)>,
    /// Each balance's last value while its holder held it.
    values: [Option<i64>; 2],
}

/// The family's balance emit on one branch.
#[derive(Default)]
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// Each projection index's tag, for the segment under way.
    names: Vec<Option<Arc<str>>>,
    /// By actor id.
    states: Vec<Actor>,
    last: Option<i32>,
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            ..Self::default()
        }
    }

    /// The emit going on after a cut of the publication `directory` reads.
    pub fn resume(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.states = vec![Actor::default(); directory.actors.len()];
        for (id, state) in emit.states.iter_mut().enumerate() {
            for (at, measure) in MEASURES.into_iter().enumerate() {
                state.keys[at] = directory
                    .key(id as u64, measure, None)
                    .ok_or_else(|| dashboard_core::invalid("A balance actor lost a key"))?;
            }
        }
        emit.load(bytes)?;
        Ok(emit)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last.filter(|_| !adjacent) {
            // An unread hour ends every run; the next values start their segments.
            self.carry.close(writer, last)?;
        }
        self.last = Some(at);
        for (index, tag) in hour.named {
            let index = index as usize;
            if index >= self.names.len() {
                self.names.resize(index + 1, None);
            }
            self.names[index] = Some(tag);
        }
        let mut offered = Vec::with_capacity(hour.countries.len());
        for country in hour.countries {
            let tag = self
                .names
                .get(country.index as usize)
                .cloned()
                .flatten()
                .ok_or_else(|| dashboard_core::invalid("A balance tag was never named"))?;
            offered.push((tag, country.held));
        }
        // New tags are numbered in tag order, as every balance reader orders them.
        let mut fresh: Vec<Arc<str>> = offered
            .iter()
            .filter(|(tag, held)| held.is_some() && self.actors.find(tag.as_bytes()).is_none())
            .map(|(tag, _)| Arc::clone(tag))
            .collect();
        fresh.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        fresh.dedup();
        for tag in fresh {
            self.enlist(writer, &tag, at)?;
        }
        if hour.whole {
            let kept: HashSet<u64> = offered
                .iter()
                .filter(|(_, held)| held.is_some())
                .filter_map(|(tag, _)| self.actors.find(tag.as_bytes()))
                .collect();
            for actor in 0..self.states.len() as u64 {
                if self.states[actor as usize].holder.is_some() && !kept.contains(&actor) {
                    self.release(writer, at, actor)?;
                }
            }
        }
        for (tag, held) in offered {
            match held {
                Some((holder, values)) => self.offer(writer, at, adjacent, &tag, holder, values)?,
                None => {
                    if let Some(actor) = self.actors.find(tag.as_bytes()) {
                        self.release(writer, at, actor)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Numbers `tag` from `hour` on, with both its keys.
    fn enlist(&mut self, writer: &mut Writer<'_>, tag: &str, hour: i32) -> io::Result<u64> {
        let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
        let mut keys = [0; 2];
        for (at, measure) in MEASURES.into_iter().enumerate() {
            keys[at] = self.actors.key(
                writer,
                Place {
                    actor,
                    measure,
                    dimension: None,
                },
                (RunMode::Exact, DECIMALS),
                None,
            )?;
        }
        if actor as usize >= self.states.len() {
            self.states.resize(actor as usize + 1, Actor::default());
        }
        self.states[actor as usize].keys = keys;
        Ok(actor)
    }

    fn offer(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        adjacent: bool,
        tag: &str,
        holder: (u32, u32),
        values: [Option<i64>; 2],
    ) -> io::Result<()> {
        let actor = match self.actors.find(tag.as_bytes()) {
            Some(actor) => actor,
            None => self.enlist(writer, tag, hour)?,
        };
        let continues = adjacent && self.states[actor as usize].holder == Some(holder);
        for (at, value) in values.into_iter().enumerate() {
            self.balance(writer, hour, (actor, at), holder, value, continues)?;
        }
        self.states[actor as usize].holder = Some(holder);
        Ok(())
    }

    /// One balance's value at `hour` under `holder`: a drop where it falls within its segment.
    fn balance(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        (actor, at): (u64, usize),
        (slot, country): (u32, u32),
        value: Option<i64>,
        continues: bool,
    ) -> io::Result<()> {
        let state = &mut self.states[actor as usize];
        let key = state.keys[at];
        let previous = state.values[at];
        let contiguous = continues && previous.is_some();
        if (value.is_none() && previous.is_none()) || (contiguous && previous == value) {
            return Ok(());
        }
        state.values[at] = value;
        if contiguous
            && let (Some(before), Some(after)) = (previous, value)
            && after < before
        {
            let fields = writer.cells([
                Cell::Integer(i128::from(before)),
                Cell::Integer(i128::from(after)),
            ])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key,
                native_hour: hour,
                read_hour: Some(hour),
                sequence,
                kind: DROP,
                fields,
            })?;
        }
        match value {
            Some(value) => {
                self.carry
                    .value(writer, key, hour, Some(i128::from(value)), !contiguous)?;
                if !contiguous {
                    self.carry.evidence(
                        writer,
                        key,
                        hour,
                        Some(Read {
                            holder: Some(u64::from(slot) << 32 | u64::from(country)),
                            read: None,
                            identity_first: Some(hour),
                            presented: false,
                        }),
                    )?;
                }
            }
            None => {
                self.carry.value(writer, key, hour, None, false)?;
                self.carry.evidence(writer, key, hour, None)?;
            }
        }
        Ok(())
    }

    /// Ends `actor`'s runs at `hour`: no resolved country holds its tag.
    fn release(&mut self, writer: &mut Writer<'_>, hour: i32, actor: u64) -> io::Result<()> {
        let state = &mut self.states[actor as usize];
        state.holder = None;
        for at in 0..2 {
            if state.values[at].take().is_some() {
                self.carry.value(writer, state.keys[at], hour, None, false)?;
                self.carry.evidence(writer, state.keys[at], hour, None)?;
            }
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)
    }

    /// What the emit goes on with after a cut: its last hour, each tag's holder and balances,
    /// and its open edges. A restart goes on at a segment's first hour, whose projection names
    /// its tags anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let put = |out: &mut Vec<u8>, value: Option<i64>| match value {
            Some(value) => {
                out.push(1);
                out.extend_from_slice(&value.to_le_bytes());
            }
            None => out.push(0),
        };
        match self.last {
            Some(last) => {
                out.push(1);
                out.extend_from_slice(&last.to_le_bytes());
            }
            None => out.push(0),
        }
        out.extend_from_slice(&(self.states.len() as u64).to_le_bytes());
        for state in &self.states {
            match state.holder {
                Some((slot, id)) => {
                    out.push(1);
                    out.extend_from_slice(&slot.to_le_bytes());
                    out.extend_from_slice(&id.to_le_bytes());
                }
                None => out.push(0),
            }
            for value in state.values {
                put(out, value);
            }
        }
        self.carry.save(out)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        let invalid = || dashboard_core::invalid("Saved balance emit is not valid");
        let mut d = Decode::new(bytes);
        self.last = match d.u8()? {
            0 => None,
            1 => Some(d.i32()?),
            _ => return Err(invalid()),
        };
        if d.u64()? != self.states.len() as u64 {
            return Err(invalid());
        }
        for state in &mut self.states {
            state.holder = match d.u8()? {
                0 => None,
                1 => Some((d.u32()?, d.u32()?)),
                _ => return Err(invalid()),
            };
            for value in &mut state.values {
                *value = match d.u8()? {
                    0 => None,
                    1 => Some(i64::from_le_bytes(d.array()?)),
                    _ => return Err(invalid()),
                };
            }
        }
        let at = d.position();
        self.carry = Carry::load(&bytes[at..])?;
        Ok(())
    }
}
