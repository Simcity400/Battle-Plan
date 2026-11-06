//! Experience's emit: the experience scan the row path steps ([`Reads`]), stepped hour by hour
//! over the joins the facts hand on ([`super::facts`]), and each read it publishes kept as runs
//! on the family's series publication. A country is its tag with its canonical slot and id;
//! its keys are the supplying read's hour, banked, gain and change by branch, and whether the
//! read starts its stretch, each with its holder evidence (slot and country id, read hour, and
//! the first hour of the segment the holder keeps), presented as an event where it changes.
//! Every published read starts its runs, equal to the runs before or not, as the row path's
//! did; a read's measures go on until the next, an unread hour or the country leaving its tag.

use super::facts::{self, Amounts};
use dashboard_base::products::country_experience::{Joins, Moved, RawRead, Reads};
use dashboard_series::{
    Cell, RunMode, Writer,
    carry::{Carry, Read},
    codec::Decode,
    family::{ACTORS, Actors, Directory, Place},
};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// The supplying read's hour.
pub const READ_HOUR: u64 = 0;
/// Banked, gain and change, each by army, navy and air.
pub const MEASURES: [[u64; 3]; 3] = [[1, 2, 3], [4, 5, 6], [7, 8, 9]];
/// Whether the supplying read starts its country's stretch: 1, else 0.
pub const STARTS: u64 = 10;
/// A country's keys, by measure.
const KEYS: usize = 11;
/// Experience units; the read hour and stretch start share the scale.
pub const DECIMALS: u8 = 0;

const BRANCHES: [&str; 3] = ["army", "navy", "air"];
const MODES: [&str; 3] = ["banked", "gain", "change"];

/// The measure a reply names: `experienceRead`, or a branch and mode such as `army-banked`.
pub fn measure(name: &str) -> Option<u64> {
    if name == "experienceRead" {
        return Some(READ_HOUR);
    }
    let (branch, mode) = name.split_once('-')?;
    let branch = BRANCHES.iter().position(|held| *held == branch)?;
    let mode = MODES.iter().position(|held| *held == mode)?;
    Some(MEASURES[mode][branch])
}

/// The name of the actor of `tag`'s country `(slot, id)`.
pub fn actor_name(tag: &str, (slot, id): (u32, u32)) -> io::Result<String> {
    Ok(serde_json::to_string(&(tag, slot, id))?)
}

/// The family scalar present once `actor`'s `measure` has held a value.
pub fn held(actor: u64, measure: u64) -> io::Result<u64> {
    actor
        .checked_mul(KEYS as u64)
        .and_then(|first| first.checked_add(measure))
        .filter(|&id| id < ACTORS)
        .ok_or_else(|| invalid("Experience actors exceed the family's scalars"))
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// A key's holder: slot, country id, read hour, and its segment's first hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Holder {
    slot: u32,
    country: u32,
    read: i32,
    first: i32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Actor {
    keys: [u64; KEYS],
    holders: [Option<Holder>; KEYS],
    /// A bit by measure: its key has held a value ([`held`]).
    held: u16,
}

/// The joins at the hour the emit has reached.
#[derive(Default)]
struct Joined {
    holders: BTreeMap<(u32, u32), Arc<str>>,
    slots: HashMap<u32, Vec<(u32, u32)>>,
    banked: HashMap<u32, Amounts>,
    gained: HashMap<u32, Amounts>,
}

impl Joins for Joined {
    fn holders(&self) -> &BTreeMap<(u32, u32), Arc<str>> {
        &self.holders
    }

    fn at(&self, slot: u32) -> &[(u32, u32)] {
        self.slots.get(&slot).map_or(&[][..], Vec::as_slice)
    }

    fn banked(&self, slot: u32) -> Option<[i64; 3]> {
        self.banked.get(&slot).copied()
    }

    fn gained(&self, slot: u32) -> Option<[i64; 3]> {
        self.gained.get(&slot).copied()
    }
}

/// Takes the hour's `values` into `held`, every slot when `whole`, and lists the slots whose
/// value changed in `dirty`.
fn apply(
    held: &mut HashMap<u32, Amounts>,
    whole: bool,
    values: Vec<(u32, Option<Amounts>)>,
    dirty: &mut Vec<u32>,
) {
    if whole {
        let next: HashMap<u32, Amounts> = values
            .into_iter()
            .filter_map(|(slot, value)| Some((slot, value?)))
            .collect();
        dirty.extend(
            held.iter()
                .filter(|(slot, value)| next.get(slot) != Some(value))
                .map(|(&slot, _)| slot),
        );
        dirty.extend(
            next.iter()
                .filter(|(slot, value)| held.get(slot) != Some(value))
                .map(|(&slot, _)| slot),
        );
        *held = next;
        return;
    }
    for (slot, value) in values {
        let changed = match value {
            Some(value) => held.insert(slot, value) != Some(value),
            None => held.remove(&slot).is_some(),
        };
        if changed {
            dirty.push(slot);
        }
    }
}

/// The family's emit on one branch.
#[derive(Default)]
pub struct Emit {
    actors: Actors,
    carry: Carry,
    reads: Reads,
    joined: Joined,
    /// Each projection index's tag, for the segment under way.
    names: Vec<Option<Arc<str>>>,
    /// Each country's actor, by its tag and country.
    outputs: HashMap<(Arc<str>, (u32, u32)), u64>,
    /// By actor id.
    states: Vec<Actor>,
    last: Option<i32>,
    /// The next hour reconciles the scan's outputs with joins read anew.
    fresh: bool,
    dirty: Vec<u32>,
    removed: Vec<(Arc<str>, (u32, u32))>,
    published: Vec<(Arc<str>, (u32, u32), RawRead)>,
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            fresh: true,
            ..Self::default()
        }
    }

    /// The emit going on after a cut of the publication `directory` reads.
    pub fn resume(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.states = vec![Actor::default(); directory.actors.len()];
        for (id, (state, actor)) in emit.states.iter_mut().zip(&directory.actors).enumerate() {
            for (measure, key) in state.keys.iter_mut().enumerate() {
                *key = directory
                    .key(id as u64, measure as u64, None)
                    .ok_or_else(|| invalid("An experience actor lost a key"))?;
                if directory.scalar(held(id as u64, measure as u64)?).is_some() {
                    state.held |= 1 << measure;
                }
            }
            let (tag, slot, country): (String, u32, u32) = serde_json::from_slice(&actor.name)
                .map_err(|_| invalid("An experience actor's name is not its country"))?;
            emit.outputs
                .insert((Arc::from(tag), (slot, country)), id as u64);
        }
        emit.load(directory, bytes)?;
        Ok(emit)
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer, hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last.filter(|_| !adjacent) {
            // An unread hour ends every run; the next reads start their segments.
            self.carry.close(writer, last)?;
            for state in &mut self.states {
                state.holders = [None; KEYS];
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
        self.dirty.clear();
        self.removed.clear();
        let roster = hour.holders.is_some();
        if let Some(holders) = hour.holders {
            let mut next = BTreeMap::new();
            for (identity, index) in holders {
                let tag = self
                    .names
                    .get(index as usize)
                    .cloned()
                    .flatten()
                    .ok_or_else(|| invalid("An experience country was never named"))?;
                next.insert(identity, tag);
            }
            self.removed.extend(
                self.joined
                    .holders
                    .iter()
                    .filter(|(identity, tag)| next.get(*identity) != Some(*tag))
                    .map(|(&identity, tag)| (Arc::clone(tag), identity)),
            );
            self.joined.slots.clear();
            for &identity in next.keys() {
                self.joined
                    .slots
                    .entry(identity.0)
                    .or_default()
                    .push(identity);
            }
            self.dirty.extend(next.keys().map(|identity| identity.0));
            self.joined.holders = next;
        }
        apply(
            &mut self.joined.banked,
            hour.whole,
            hour.banked,
            &mut self.dirty,
        );
        apply(
            &mut self.joined.gained,
            hour.whole,
            hour.gained,
            &mut self.dirty,
        );
        self.dirty.sort_unstable();
        self.dirty.dedup();
        let rebuilt = std::mem::take(&mut self.fresh);
        let published = &mut self.published;
        published.clear();
        self.reads.step(
            &self.joined,
            at,
            Moved {
                dirty: &self.dirty,
                removed: &self.removed,
                roster,
                rebuilt,
                read: hour.read,
            },
            |tag, identity, read| {
                // A read held on from an earlier hour changes none of its keys.
                if read.first_hour == at {
                    published.push((Arc::clone(tag), identity, read));
                }
                Ok(())
            },
        )?;
        let published = std::mem::take(&mut self.published);
        for (tag, identity, read) in &published {
            self.offer(writer, at, tag, *identity, read)?;
        }
        self.published = published;
        self.removed.clear();
        self.removed.extend_from_slice(self.reads.removed());
        let removed = std::mem::take(&mut self.removed);
        for (tag, identity) in &removed {
            self.close(writer, at, tag, *identity)?;
        }
        self.removed = removed;
        Ok(())
    }

    /// Keeps `read`, which `tag`'s country `identity` publishes from `hour`.
    fn offer(
        &mut self,
        writer: &mut Writer,
        hour: i32,
        tag: &Arc<str>,
        identity: (u32, u32),
        read: &RawRead,
    ) -> io::Result<()> {
        let output = (Arc::clone(tag), identity);
        let actor = match self.outputs.get(&output) {
            Some(&actor) => actor,
            None => {
                let name = actor_name(tag, identity)?;
                let actor = self.actors.actor(writer, name.as_bytes(), hour)?;
                let mut keys = [0; KEYS];
                for (measure, key) in keys.iter_mut().enumerate() {
                    *key = self.actors.key(
                        writer,
                        Place {
                            actor,
                            measure: measure as u64,
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
                self.outputs.insert(output, actor);
                actor
            }
        };
        let joined = !read.starts_segment;
        let holder = (identity.0, identity.1, read.read_hour);
        self.record(
            writer,
            (actor, READ_HOUR),
            hour,
            Some(i128::from(read.read_hour)),
            joined,
            holder,
        )?;
        for (row, values) in MEASURES.iter().zip(read.values()) {
            for (&measure, value) in row.iter().zip(values) {
                self.record(writer, (actor, measure), hour, value, joined, holder)?;
            }
        }
        self.record(
            writer,
            (actor, STARTS),
            hour,
            Some(i128::from(read.starts_segment)),
            joined,
            holder,
        )
    }

    /// Ends every key of `tag`'s country `identity` before `hour`: it left the outputs.
    fn close(
        &mut self,
        writer: &mut Writer,
        hour: i32,
        tag: &Arc<str>,
        identity: (u32, u32),
    ) -> io::Result<()> {
        let Some(&actor) = self.outputs.get(&(Arc::clone(tag), identity)) else {
            return Ok(());
        };
        let state = &mut self.states[actor as usize];
        for at in 0..KEYS {
            self.carry
                .value(writer, state.keys[at], hour, None, false)?;
            self.carry.evidence(writer, state.keys[at], hour, None)?;
            state.holders[at] = None;
        }
        Ok(())
    }

    /// One key's value at `hour`, read anew there: a holder continuing the key's segment keeps
    /// its first hour; any other starts one, with a break.
    fn record(
        &mut self,
        writer: &mut Writer,
        (actor, measure): (u64, u64),
        hour: i32,
        value: Option<i128>,
        joined: bool,
        (slot, country, read): (u32, u32, i32),
    ) -> io::Result<()> {
        let state = &mut self.states[actor as usize];
        let at = measure as usize;
        let key = state.keys[at];
        let Some(value) = value else {
            self.carry.value(writer, key, hour, None, false)?;
            self.carry.evidence(writer, key, hour, None)?;
            state.holders[at] = None;
            return Ok(());
        };
        if state.held & 1 << at == 0 {
            writer.catalogue(held(actor, measure)?, Cell::Boolean(true))?;
            state.held |= 1 << at;
        }
        let held = state.holders[at];
        let same = joined && held.is_some_and(|held| (held.slot, held.country) == (slot, country));
        self.carry.supplied(writer, key, hour, value, !same)?;
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

    /// What the emit goes on with after a cut: its last hour, each key's holder, the scan's
    /// reads and its open edges. A restart goes on at a segment's first hour, whose projection
    /// names its countries and hands on every join anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        match self.last {
            Some(last) => {
                out.push(1);
                out.extend_from_slice(&last.to_le_bytes());
            }
            None => out.push(0),
        }
        out.extend_from_slice(&(self.states.len() as u64).to_le_bytes());
        for state in &self.states {
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
        let reads = serde_json::to_vec(&self.reads)?;
        out.extend_from_slice(&(reads.len() as u64).to_le_bytes());
        out.extend_from_slice(&reads);
        self.carry.save(out)
    }

    fn load(&mut self, directory: &Directory, bytes: &[u8]) -> io::Result<()> {
        let invalid = || invalid("Saved experience emit is not valid");
        let mut d = Decode::new(bytes);
        self.last = match d.u8()? {
            0 => None,
            1 => Some(d.i32()?),
            _ => return Err(invalid()),
        };
        if d.u64()? != directory.actors.len() as u64 {
            return Err(invalid());
        }
        for state in &mut self.states {
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
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        self.reads = serde_json::from_slice(d.bytes(length)?).map_err(|_| invalid())?;
        let at = d.position();
        self.carry = Carry::load(&bytes[at..])?;
        Ok(())
    }
}
