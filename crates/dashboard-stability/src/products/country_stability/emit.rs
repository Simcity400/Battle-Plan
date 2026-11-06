//! Stability's measure emit: each country's supplying read hour, stability, war support and
//! maximum command power as runs on the family's series publication, each key with its
//! holder evidence (slot and country id, read hour, and the first hour of the segment the
//! holder keeps), presented as an event where it changes.
//!
//! A country is offered when its holder, read or measures changed. A measure's run goes on
//! while its holder keeps its tag and its value stays known; anything else starts a segment.
//! A tag leaving the countries with a holder and inputs ends its runs; so does an unread hour.

use super::facts;
use dashboard_series::{
    RunMode, Writer,
    carry::{Carry, Read},
    codec::Decode,
    family::{Actors, Directory, Place},
};
use std::{collections::HashSet, io, sync::Arc};

/// The family's measure keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// The supplying inputs read's hour, then the three displayed measures.
pub const READ_HOUR: u64 = 0;
pub const MEASURES: [u64; 3] = [1, 2, 3];
/// Fixed5, as every displayed measure; the read hour shares the scale.
pub const DECIMALS: u8 = dashboard_base::products::country_measure::MEASURE_DECIMALS;

/// The measure names a reply asks by.
pub fn measure(name: &str) -> Option<u64> {
    Some(match name {
        "readHour" => READ_HOUR,
        "stability" => MEASURES[0],
        "warSupport" => MEASURES[1],
        "maxCommandPower" => MEASURES[2],
        _ => return None,
    })
}

/// A key's holder: slot, country id, read hour, and its segment's first hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Holder {
    slot: u32,
    country: u32,
    read: i32,
    first: i32,
}

/// What a country's last offer held.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Track {
    identity: Option<(u32, u32)>,
    values: [Option<i64>; 3],
    read: Option<i32>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Actor {
    track: Track,
    keys: [u64; 4],
    holders: [Option<Holder>; 4],
}

/// The family's measure emit on one branch.
#[derive(Default)]
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// Each projection index's tag, for the segment under way.
    names: Vec<Option<Arc<str>>>,
    /// By actor id.
    states: Vec<Actor>,
    /// Tags with a current holder, inputs and a read.
    active: HashSet<Arc<str>>,
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
            for (at, key) in std::iter::once(READ_HOUR).chain(MEASURES).enumerate() {
                state.keys[at] = directory
                    .key(id as u64, key, None)
                    .ok_or_else(|| dashboard_core::invalid("A stability actor lost a key"))?;
            }
        }
        emit.load(directory, bytes)?;
        Ok(emit)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last.filter(|_| !adjacent) {
            // An unread hour ends every run; the next offers start their segments.
            self.carry.close(writer, last)?;
            for state in &mut self.states {
                state.holders = [None; 4];
            }
        }
        self.last = Some(at);
        for (index, tag) in hour.named {
            let index = index as usize;
            if index >= self.names.len() {
                self.names.resize(index + 1, None);
            }
            self.names[index] = Some(tag);
        }
        let name = |names: &[Option<Arc<str>>], index: u32| {
            names
                .get(index as usize)
                .cloned()
                .flatten()
                .ok_or_else(|| dashboard_core::invalid("A stability country was never named"))
        };
        let mut removed = Vec::new();
        if let Some(valid) = hour.valid {
            let valid: HashSet<Arc<str>> = valid
                .into_iter()
                .map(|index| name(&self.names, index))
                .collect::<io::Result<_>>()?;
            removed.extend(self.active.difference(&valid).cloned());
            for tag in &removed {
                if let Some(actor) = self.actors.find(tag.as_bytes()) {
                    self.states[actor as usize].track.identity = None;
                }
            }
            self.active = valid;
        }
        for country in hour.countries {
            let tag = name(&self.names, country.index)?;
            self.offer(writer, at, adjacent, &tag, country)?;
        }
        for tag in removed {
            let Some(actor) = self.actors.find(tag.as_bytes()) else {
                continue;
            };
            let state = &mut self.states[actor as usize];
            for at_key in 0..4 {
                self.carry
                    .value(writer, state.keys[at_key], at, None, false)?;
                self.carry.evidence(writer, state.keys[at_key], at, None)?;
                state.holders[at_key] = None;
            }
        }
        Ok(())
    }

    fn offer(
        &mut self,
        writer: &mut Writer,
        hour: i32,
        adjacent: bool,
        tag: &str,
        country: facts::Country,
    ) -> io::Result<()> {
        let actor = match self.actors.find(tag.as_bytes()) {
            Some(actor) => actor,
            None => {
                let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
                let mut keys = [0; 4];
                for (at, measure) in std::iter::once(READ_HOUR).chain(MEASURES).enumerate() {
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
                actor
            }
        };
        let track = self.states[actor as usize].track;
        let identity = country.holder;
        let continues = adjacent && track.identity == Some(identity);
        if continues && track.values == country.values && track.read == Some(country.read) {
            return Ok(());
        }
        let joined = track.values.map(|value| continues && value.is_some());
        let changed: [bool; 4] = std::array::from_fn(|index| {
            !continues
                || track.read != Some(country.read)
                || index > 0 && track.values[index - 1] != country.values[index - 1]
        });
        let holder = (identity.0, identity.1, country.read);
        if changed[0] {
            self.record(
                writer,
                (actor, 0),
                hour,
                Some(i64::from(country.read)),
                adjacent,
                holder,
            )?;
        }
        for index in 0..3 {
            if changed[index + 1] {
                self.record(
                    writer,
                    (actor, index + 1),
                    hour,
                    country.values[index],
                    joined[index],
                    holder,
                )?;
            }
        }
        self.states[actor as usize].track = Track {
            identity: Some(identity),
            values: country.values,
            read: Some(country.read),
        };
        Ok(())
    }

    /// One key's value at `hour`: a holder continuing the key's segment keeps its first hour;
    /// any other starts one, with a break.
    fn record(
        &mut self,
        writer: &mut Writer,
        (actor, at): (u64, usize),
        hour: i32,
        value: Option<i64>,
        joined: bool,
        (slot, country, read): (u32, u32, i32),
    ) -> io::Result<()> {
        let state = &mut self.states[actor as usize];
        let key = state.keys[at];
        let Some(value) = value else {
            self.carry.value(writer, key, hour, None, false)?;
            self.carry.evidence(writer, key, hour, None)?;
            state.holders[at] = None;
            return Ok(());
        };
        let held = state.holders[at];
        let same = joined && held.is_some_and(|held| (held.slot, held.country) == (slot, country));
        self.carry
            .value(writer, key, hour, Some(i128::from(value)), !same)?;
        let next = Holder {
            slot,
            country,
            read,
            first: match held.filter(|_| same) {
                Some(held) => held.first,
                None => hour,
            },
        };
        if held != Some(next) {
            self.carry.evidence(
                writer,
                key,
                hour,
                Some(Read {
                    holder: Some(u64::from(slot) << 32 | u64::from(country)),
                    read: Some(read),
                    identity_first: Some(next.first),
                    presented: true,
                }),
            )?;
            state.holders[at] = Some(next);
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next.
    pub fn cut(&mut self, writer: &mut Writer, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)
    }

    /// What the emit goes on with after a cut: each country's track and keys' holders, the
    /// active tags, and its open edges. A restart goes on at a segment's first hour, whose
    /// projection names its countries anew.
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
        let mut active: Vec<&[u8]> = self.active.iter().map(|tag| tag.as_bytes()).collect();
        active.sort_unstable();
        out.extend_from_slice(&(active.len() as u64).to_le_bytes());
        for tag in active {
            out.extend_from_slice(&(tag.len() as u32).to_le_bytes());
            out.extend_from_slice(tag);
        }
        out.extend_from_slice(&(self.states.len() as u64).to_le_bytes());
        for state in &self.states {
            match state.track.identity {
                Some((slot, id)) => {
                    out.push(1);
                    out.extend_from_slice(&slot.to_le_bytes());
                    out.extend_from_slice(&id.to_le_bytes());
                }
                None => out.push(0),
            }
            for value in state.track.values {
                put(out, value);
            }
            put(out, state.track.read.map(i64::from));
            for holder in state.holders {
                match holder {
                    Some(holder) => {
                        out.push(1);
                        for word in [holder.slot, holder.country] {
                            out.extend_from_slice(&word.to_le_bytes());
                        }
                        for hour in [holder.read, holder.first] {
                            out.extend_from_slice(&hour.to_le_bytes());
                        }
                    }
                    None => out.push(0),
                }
            }
        }
        self.carry.save(out)
    }

    fn load(&mut self, directory: &Directory, bytes: &[u8]) -> io::Result<()> {
        let invalid = || dashboard_core::invalid("Saved stability emit is not valid");
        let mut d = Decode::new(bytes);
        let value = |d: &mut Decode<'_>| -> io::Result<Option<i64>> {
            Ok(match d.u8()? {
                0 => None,
                1 => Some(i64::from_le_bytes(d.array()?)),
                _ => return Err(invalid()),
            })
        };
        self.last = match d.u8()? {
            0 => None,
            1 => Some(d.i32()?),
            _ => return Err(invalid()),
        };
        for _ in 0..d.u64()? {
            let length = d.u32()? as usize;
            let tag = std::str::from_utf8(d.bytes(length)?).map_err(|_| invalid())?;
            self.active.insert(Arc::from(tag));
        }
        if d.u64()? != directory.actors.len() as u64 {
            return Err(invalid());
        }
        for state in &mut self.states {
            state.track.identity = match d.u8()? {
                0 => None,
                1 => Some((d.u32()?, d.u32()?)),
                _ => return Err(invalid()),
            };
            for slot in &mut state.track.values {
                *slot = value(&mut d)?;
            }
            state.track.read = value(&mut d)?
                .map(|read| i32::try_from(read).map_err(|_| invalid()))
                .transpose()?;
            for holder in &mut state.holders {
                *holder = match d.u8()? {
                    0 => None,
                    1 => Some(Holder {
                        slot: d.u32()?,
                        country: d.u32()?,
                        read: d.i32()?,
                        first: d.i32()?,
                    }),
                    _ => return Err(invalid()),
                };
            }
        }
        let at = d.position();
        self.carry = Carry::load(&bytes[at..])?;
        Ok(())
    }
}
