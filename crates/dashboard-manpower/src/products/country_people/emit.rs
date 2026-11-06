//! The manpower family's emit: each country's manpower operands as runs on the family's series
//! publication, each key with its holder evidence (slot and country id, and the first hour of
//! the segment the holder keeps).
//!
//! A country's keys: its residual manpower, applied recruitment ratio and its read hour, its
//! four effective recruitment modifiers and its five deployed totals; for each state it
//! controls (the key's dimension), the state's available, locked and total manpower, its four
//! recruitment inputs and whether the country cores it; and its controlled states as recorded,
//! in order and with any repeats, as events written where they change and where its holder's
//! segment starts. A value's run goes on while its holder keeps the tag and the value stays
//! known; anything else starts a segment. A state leaving the roster, a tag leaving the
//! countries and an unread hour end their runs.

use super::facts;
use dashboard_base::products::country_people::{Operands, StateOperands};
use dashboard_series::{
    Cell, Event, KeyHandle, RunMode, Writer,
    carry::{Carry, Read},
    codec::Decode,
    family::{Actors, Directory, Place as Key},
};
use replay_data::codec::{Decoder, put_varint};
use std::{
    collections::{BTreeMap, HashSet},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;

/// A country's keys: residual, applied ratio, its read hour, the effective modifiers and the
/// deployed totals.
pub const RESIDUAL: u64 = 0;
pub const APPLIED: u64 = 1;
pub const READ_HOUR: u64 = 2;
pub const EFFECTIVE: [u64; 4] = [3, 4, 5, 6];
pub const DEPLOYED: [u64; 5] = [7, 8, 9, 10, 11];
/// A country's controlled states, as [`kind`] events.
pub const ROSTER: u64 = 12;
/// The country's keys in [`values`]' order.
pub const COUNTRY: [u64; 12] = [
    RESIDUAL,
    APPLIED,
    READ_HOUR,
    EFFECTIVE[0],
    EFFECTIVE[1],
    EFFECTIVE[2],
    EFFECTIVE[3],
    DEPLOYED[0],
    DEPLOYED[1],
    DEPLOYED[2],
    DEPLOYED[3],
    DEPLOYED[4],
];

/// Events of a country's [`ROSTER`].
pub mod kind {
    /// `[Unsigned state, …]`: its controlled states in their recorded order, repeats kept.
    pub const ROSTER: u64 = 0;
    /// `[]`: which states it controls is unknown.
    pub const UNKNOWN: u64 = 1;
}

/// A controlled state's keys, by state: its available, locked and total manpower, the four
/// recruitment inputs, and whether the country cores it (1 or 0).
pub const AVAILABLE: u64 = 21;
pub const LOCKED: u64 = 22;
pub const TOTAL: u64 = 23;
pub const RESOURCES: [u64; 4] = [24, 25, 26, 27];
pub const CORE: u64 = 28;
/// A state's keys in [`state_values`]' order.
pub const STATE: [u64; 8] = [
    AVAILABLE,
    LOCKED,
    TOTAL,
    RESOURCES[0],
    RESOURCES[1],
    RESOURCES[2],
    RESOURCES[3],
    CORE,
];

/// Every value is a raw integer.
pub const DECIMALS: u8 = 0;

/// The country keys' values of `operands`, in [`COUNTRY`]'s order.
pub fn values(operands: &Operands) -> [Option<i64>; 12] {
    let effective = |at: usize| operands.effective.map(|values| values[at]);
    let deployed = |at: usize| operands.deployed[at].map(i64::from);
    [
        Some(i64::from(operands.residual)),
        operands.applied,
        operands.read_hour.map(i64::from),
        effective(0),
        effective(1),
        effective(2),
        effective(3),
        deployed(0),
        deployed(1),
        deployed(2),
        deployed(3),
        deployed(4),
    ]
}

/// A controlled state's keys' values, in [`STATE`]'s order.
pub fn state_values(row: Option<&StateOperands>) -> [Option<i64>; 8] {
    let resource = |at: usize| row.and_then(|row| row.resources).map(|values| values[at]);
    [
        row.map(|row| i64::from(row.available)),
        row.and_then(|row| row.locked).map(i64::from),
        row.and_then(|row| row.total).map(i64::from),
        resource(0),
        resource(1),
        resource(2),
        resource(3),
        row.and_then(|row| row.core).map(i64::from),
    ]
}

/// A key of a country: its measure, and the state it is of for a state's.
pub type Place = (u64, Option<u64>);

/// Keys and their values, none where a key holds none now, packed as the walk's items hold
/// them: the measure with whether a state and a value follow, the state, the value zigzagged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Values(Vec<u8>);

impl Values {
    pub fn push(&mut self, (measure, state): Place, value: Option<i64>) {
        let flags = u64::from(state.is_some()) << 1 | u64::from(value.is_some());
        put_varint(&mut self.0, u128::from(measure) << 2 | u128::from(flags));
        if let Some(state) = state {
            put_varint(&mut self.0, u128::from(state));
        }
        if let Some(value) = value {
            put_varint(&mut self.0, u128::from((value << 1 ^ value >> 63) as u64));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn shrink(&mut self) {
        self.0.shrink_to_fit();
    }

    pub fn heap(&self) -> usize {
        self.0.capacity()
    }

    /// Each key and its value, in the order pushed.
    pub fn each(&self, mut f: impl FnMut(Place, Option<i64>) -> io::Result<()>) -> io::Result<()> {
        fn word(bytes: &mut Decoder<'_>) -> io::Result<u64> {
            u64::try_from(bytes.varint()?)
                .map_err(|_| dashboard_core::invalid("A manpower value is malformed"))
        }
        let mut bytes = Decoder::new(&self.0);
        while bytes.remaining() > 0 {
            let head = word(&mut bytes)?;
            let state = if head & 2 != 0 { Some(word(&mut bytes)?) } else { None };
            let value = if head & 1 != 0 {
                let raw = word(&mut bytes)?;
                Some((raw >> 1) as i64 ^ -((raw & 1) as i64))
            } else {
                None
            };
            f((head >> 2, state), value)?;
        }
        Ok(())
    }
}

/// A key's holder: slot, country id, and its segment's first hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Holder {
    slot: u32,
    country: u32,
    first: i32,
}

/// What a country's last offer held.
#[derive(Clone, Debug, Default)]
struct Actor {
    identity: Option<(u32, u32)>,
    /// Every key's value as last offered.
    values: BTreeMap<Place, i64>,
    /// Each open key's holder.
    holders: BTreeMap<Place, Holder>,
    /// Its controlled states as its holder's segment last wrote them; none before it writes.
    roster: Option<Option<Vec<u32>>>,
}

/// The family's emit on one branch.
#[derive(Default)]
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// Each projection index's tag, for the segment under way.
    names: Vec<Option<Arc<str>>>,
    /// By actor id.
    states: Vec<Actor>,
    /// Tags with a current holder.
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
        emit.load(bytes)?;
        Ok(emit)
    }

    fn key(&mut self, writer: &mut Writer<'_>, actor: u64, (measure, dimension): Place) -> io::Result<KeyHandle> {
        let place = Key {
            actor,
            measure,
            dimension,
        };
        match self.actors.held(place) {
            Some(key) => Ok(key),
            None => self
                .actors
                .key(writer, place, (RunMode::Exact, DECIMALS), None),
        }
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last.filter(|_| !adjacent) {
            // An unread hour ends every run; the next offers start their segments.
            self.carry.close(writer, last)?;
            for state in &mut self.states {
                state.holders.clear();
                state.values.clear();
                state.roster = None;
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
                .ok_or_else(|| dashboard_core::invalid("A manpower country was never named"))
        };
        let mut removed = Vec::new();
        if let Some(valid) = hour.valid {
            let valid: HashSet<Arc<str>> = valid
                .into_iter()
                .map(|index| name(&self.names, index))
                .collect::<io::Result<_>>()?;
            removed.extend(self.active.difference(&valid).cloned());
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
            let state = std::mem::take(&mut self.states[actor as usize]);
            for place in state.values.keys() {
                let key = self.key(writer, actor, *place)?;
                self.carry.value(writer, key, at, None, false)?;
                self.carry.evidence(writer, key, at, None)?;
            }
        }
        Ok(())
    }

    fn offer(
        &mut self,
        writer: &mut Writer<'_>,
        hour: i32,
        adjacent: bool,
        tag: &str,
        country: facts::Country,
    ) -> io::Result<()> {
        let actor = match self.actors.find(tag.as_bytes()) {
            Some(actor) => actor,
            None => {
                let actor = self.actors.actor(writer, tag.as_bytes(), hour)?;
                if actor as usize >= self.states.len() {
                    self.states.resize(actor as usize + 1, Actor::default());
                }
                actor
            }
        };
        let (slot, id) = country.holder;
        let mut state = std::mem::take(&mut self.states[actor as usize]);
        let continues = adjacent && state.identity == Some((slot, id));
        if !continues {
            state.holders.clear();
            state.roster = None;
        }
        match country.roster {
            Some(roster) => {
                let roster = roster.map(Vec::from);
                if state.roster.as_ref() != Some(&roster) {
                    self.roster(writer, actor, hour, roster.as_deref())?;
                    state.roster = Some(roster);
                }
            }
            None if state.roster.is_none() => {
                return Err(dashboard_core::invalid(
                    "A manpower country starts its segment without its states",
                ));
            }
            None => {}
        }
        if country.whole || !continues {
            // Keys it held and does not now end; the rest run as they offer.
            let mut now = BTreeMap::new();
            country.values.each(|place, value| {
                if let Some(value) = value {
                    now.insert(place, value);
                }
                Ok(())
            })?;
            let ended: Vec<Place> = state
                .values
                .keys()
                .filter(|place| !now.contains_key(place))
                .copied()
                .collect();
            for place in ended {
                self.end(writer, actor, &mut state, place, hour)?;
            }
            for (place, value) in now {
                self.set(writer, actor, &mut state, (slot, id), (place, value), hour, continues)?;
            }
        } else {
            country.values.each(|place, value| match value {
                Some(value) => self.set(writer, actor, &mut state, (slot, id), (place, value), hour, true),
                None => self.end(writer, actor, &mut state, place, hour),
            })?;
        }
        state.identity = Some((slot, id));
        self.states[actor as usize] = state;
        Ok(())
    }

    /// The country's controlled states from `hour`; none where unknown.
    fn roster(
        &mut self,
        writer: &mut Writer<'_>,
        actor: u64,
        hour: i32,
        roster: Option<&[u32]>,
    ) -> io::Result<()> {
        let key = self.key(writer, actor, (ROSTER, None))?;
        let (kind, fields) = match roster {
            Some(states) => (
                kind::ROSTER,
                writer.cells(states.iter().map(|&state| Cell::Unsigned(u64::from(state))))?,
            ),
            None => (kind::UNKNOWN, writer.cells(std::iter::empty())?),
        };
        let sequence = writer.sequence()?;
        writer.event(Event {
            key,
            native_hour: hour,
            read_hour: Some(hour),
            sequence,
            kind,
            fields,
        })
    }

    /// `place` holds no value from `hour`.
    fn end(
        &mut self,
        writer: &mut Writer<'_>,
        actor: u64,
        state: &mut Actor,
        place: Place,
        hour: i32,
    ) -> io::Result<()> {
        if state.values.remove(&place).is_none() {
            return Ok(());
        }
        state.holders.remove(&place);
        let key = self.key(writer, actor, place)?;
        self.carry.value(writer, key, hour, None, false)?;
        self.carry.evidence(writer, key, hour, None)
    }

    /// `place` holds `value` from `hour`: a value its holder's segment already holds writes
    /// nothing, one after none starts a segment.
    #[allow(clippy::too_many_arguments)]
    fn set(
        &mut self,
        writer: &mut Writer<'_>,
        actor: u64,
        state: &mut Actor,
        (slot, id): (u32, u32),
        (place, value): (Place, i64),
        hour: i32,
        continues: bool,
    ) -> io::Result<()> {
        let held = state.holders.get(&place).copied().filter(|_| continues);
        let same_value = continues && state.values.get(&place) == Some(&value);
        let holder = Holder {
            slot,
            country: id,
            first: held.map_or(hour, |held| held.first),
        };
        state.values.insert(place, value);
        state.holders.insert(place, holder);
        if same_value && held == Some(holder) {
            return Ok(());
        }
        let key = self.key(writer, actor, place)?;
        if !same_value {
            self.carry
                .value(writer, key, hour, Some(i128::from(value)), held.is_none())?;
        }
        if held != Some(holder) {
            self.carry.evidence(
                writer,
                key,
                hour,
                Some(Read {
                    holder: Some(u64::from(slot) << 32 | u64::from(id)),
                    read: None,
                    identity_first: Some(holder.first),
                    presented: true,
                }),
            )?;
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)
    }

    /// What the emit goes on with after a cut: each country's identity, values and holders, the
    /// active tags, and its open edges. A restart goes on at a segment's first hour, whose
    /// projection names its countries anew.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let put_place = |out: &mut Vec<u8>, (measure, dimension): Place| {
            out.extend_from_slice(&measure.to_le_bytes());
            match dimension {
                Some(dimension) => {
                    out.push(1);
                    out.extend_from_slice(&dimension.to_le_bytes());
                }
                None => out.push(0),
            }
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
            match state.identity {
                Some((slot, id)) => {
                    out.push(1);
                    out.extend_from_slice(&slot.to_le_bytes());
                    out.extend_from_slice(&id.to_le_bytes());
                }
                None => out.push(0),
            }
            out.extend_from_slice(&(state.values.len() as u64).to_le_bytes());
            for (&place, &value) in &state.values {
                put_place(out, place);
                out.extend_from_slice(&value.to_le_bytes());
            }
            out.extend_from_slice(&(state.holders.len() as u64).to_le_bytes());
            for (&place, holder) in &state.holders {
                put_place(out, place);
                out.extend_from_slice(&holder.slot.to_le_bytes());
                out.extend_from_slice(&holder.country.to_le_bytes());
                out.extend_from_slice(&holder.first.to_le_bytes());
            }
            match &state.roster {
                None => out.push(0),
                Some(None) => out.push(1),
                Some(Some(states)) => {
                    out.push(2);
                    out.extend_from_slice(&(states.len() as u64).to_le_bytes());
                    for state in states {
                        out.extend_from_slice(&state.to_le_bytes());
                    }
                }
            }
        }
        self.carry.save(out)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        let invalid = || dashboard_core::invalid("Saved manpower emit is not valid");
        let mut d = Decode::new(bytes);
        let place = |d: &mut Decode<'_>| -> io::Result<Place> {
            let measure = d.u64()?;
            let dimension = match d.u8()? {
                0 => None,
                1 => Some(d.u64()?),
                _ => return Err(invalid()),
            };
            Ok((measure, dimension))
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
        if d.u64()? != self.states.len() as u64 {
            return Err(invalid());
        }
        for state in &mut self.states {
            state.identity = match d.u8()? {
                0 => None,
                1 => Some((d.u32()?, d.u32()?)),
                _ => return Err(invalid()),
            };
            for _ in 0..d.u64()? {
                let at = place(&mut d)?;
                state.values.insert(at, i64::from_le_bytes(d.array()?));
            }
            for _ in 0..d.u64()? {
                let at = place(&mut d)?;
                let holder = Holder {
                    slot: d.u32()?,
                    country: d.u32()?,
                    first: d.i32()?,
                };
                state.holders.insert(at, holder);
            }
            state.roster = match d.u8()? {
                0 => None,
                1 => Some(None),
                2 => {
                    let count = d.u64()?;
                    let mut states = Vec::new();
                    for _ in 0..count {
                        states.push(d.u32()?);
                    }
                    Some(Some(states))
                }
                _ => return Err(invalid()),
            };
        }
        let at = d.position();
        self.carry = Carry::load(&bytes[at..])?;
        Ok(())
    }
}
