//! The place-names family's emit: each province's and state's name inputs as events on the
//! family's series publication, written where they change, how the name tables held their rows
//! each hour they changed, and the hours walked as a run.
//!
//! A reader takes a province's or state's inputs at an hour from its latest event by then: a
//! row's inputs, or that it has none. Those of a walked hour are the frame's whatever came
//! between, as a segment's first hour compares every row with what the emit holds.

use super::facts::{self, Change, Province, Tables};
use dashboard_base::products::place_names::{Owner, StateInputs};
use dashboard_series::{
    Cell, Event, KeyHandle, RunMode, Writer,
    carry::{Carry, Read},
    codec::Decode,
    family::{Actors, Directory, Place},
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// The actor of every key.
pub const FAMILY: u64 = u64::MAX;

/// The family's keys: the walked hours (a run of 1), the tables' holds, and each province's and
/// state's inputs, by id.
pub const OBSERVED: u64 = 0;
pub const TABLES: u64 = 1;
pub const PROVINCE: u64 = 2;
pub const STATE: u64 = 3;

/// Event kinds.
pub mod kind {
    /// `[Boolean renames complete, Text renames gap, Text states' capture mode | Null, Text
    /// states gap, Boolean reserved tag known, Text reserved tag | why]`, on TABLES.
    pub const TABLES: u64 = 0;
    /// `[Unsigned state, Text rename | Null]`.
    pub const PROVINCE: u64 = 1;
    /// `[Text custom name, Text owner name, Unsigned owner, Text | Unsigned]`: owner 0 with its
    /// tag, 1 with why it is unknown, 2 with the raw link that resolves to no country.
    pub const STATE: u64 = 2;
    /// `[]`: no row from here on.
    pub const GONE: u64 = 3;
}

/// The walked hours' evidence: every one is known, as the engine requires of a run.
const WALKED: Read = Read {
    holder: None,
    read: None,
    identity_first: None,
    presented: false,
};

fn invalid() -> io::Error {
    dashboard_core::invalid("Saved place-names emit is not valid")
}

/// The family's emit on one branch.
#[derive(Default)]
pub struct Emit {
    actors: Actors,
    carry: Carry,
    provinces: HashMap<u32, Province>,
    states: HashMap<u32, StateInputs>,
    tables: Option<Tables>,
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
        emit.load(bytes)?;
        Ok(emit)
    }

    fn key(&mut self, writer: &mut Writer<'_>, measure: u64, id: Option<u32>) -> io::Result<KeyHandle> {
        let place = Place {
            actor: FAMILY,
            measure,
            dimension: id.map(u64::from),
        };
        match self.actors.held(place) {
            Some(key) => Ok(key),
            None => self.actors.key(writer, place, (RunMode::Exact, 0), None),
        }
    }

    fn event(
        &mut self,
        writer: &mut Writer<'_>,
        (measure, id): (u64, Option<u32>),
        at: i32,
        kind: u64,
        cells: &[Cell],
    ) -> io::Result<()> {
        let key = self.key(writer, measure, id)?;
        let fields = writer.cells(cells.iter().copied())?;
        let sequence = writer.sequence()?;
        writer.event(Event {
            key,
            native_hour: at,
            read_hour: Some(at),
            sequence,
            kind,
            fields,
        })
    }

    /// Keeps one projected hour.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let at = hour.hour;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last.filter(|_| !adjacent) {
            // An hour not walked ends the walked run.
            self.carry.close(writer, last)?;
        }
        if !adjacent {
            let observed = self.key(writer, OBSERVED, None)?;
            self.carry.evidence(writer, observed, at, Some(WALKED))?;
            self.carry.value(writer, observed, at, Some(1), true)?;
        }
        self.last = Some(at);
        if let Some(tables) = hour.tables
            && self.tables.as_ref() != Some(&tables)
        {
            self.write_tables(writer, at, &tables)?;
            self.tables = Some(tables);
        }
        if hour.whole {
            let (mut held_provinces, mut held_states) = (HashSet::new(), HashSet::new());
            for change in &hour.changes {
                match change {
                    Change::Province(id, Some(_)) => held_provinces.insert(*id),
                    Change::State(id, Some(_)) => held_states.insert(*id),
                    _ => false,
                };
            }
            let mut provinces: Vec<u32> = self
                .provinces
                .keys()
                .filter(|id| !held_provinces.contains(id))
                .copied()
                .collect();
            let mut states: Vec<u32> = self
                .states
                .keys()
                .filter(|id| !held_states.contains(id))
                .copied()
                .collect();
            provinces.sort_unstable();
            states.sort_unstable();
            for id in provinces {
                self.province(writer, at, id, None)?;
            }
            for id in states {
                self.state(writer, at, id, None)?;
            }
        }
        for change in hour.changes {
            match change {
                Change::Province(id, now) => self.province(writer, at, id, now)?,
                Change::State(id, now) => self.state(writer, at, id, now)?,
            }
        }
        Ok(())
    }

    fn write_tables(&mut self, writer: &mut Writer<'_>, at: i32, tables: &Tables) -> io::Result<()> {
        let mode = match &tables.states_unrecorded {
            Some(mode) => Cell::Text(writer.text(mode)?),
            None => Cell::Null,
        };
        let (known, reserved) = match &tables.reserved {
            Ok(tag) => (true, tag),
            Err(why) => (false, why),
        };
        let cells = [
            Cell::Boolean(tables.renames_complete),
            Cell::Text(writer.text(&tables.renames_gap)?),
            mode,
            Cell::Text(writer.text(&tables.states_gap)?),
            Cell::Boolean(known),
            Cell::Text(writer.text(reserved)?),
        ];
        self.event(writer, (TABLES, None), at, kind::TABLES, &cells)
    }

    fn province(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        id: u32,
        now: Option<Province>,
    ) -> io::Result<()> {
        if self.provinces.get(&id) == now.as_ref() {
            return Ok(());
        }
        let Some(now) = now else {
            self.provinces.remove(&id);
            return self.event(writer, (PROVINCE, Some(id)), at, kind::GONE, &[]);
        };
        let rename = match &now.rename {
            Some(rename) => Cell::Text(writer.blob(&mut &rename[..], rename.len() as u64)?),
            None => Cell::Null,
        };
        let cells = [Cell::Unsigned(u64::from(now.state)), rename];
        self.event(writer, (PROVINCE, Some(id)), at, kind::PROVINCE, &cells)?;
        self.provinces.insert(id, now);
        Ok(())
    }

    fn state(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        id: u32,
        now: Option<StateInputs>,
    ) -> io::Result<()> {
        if self.states.get(&id) == now.as_ref() {
            return Ok(());
        }
        let Some(now) = now else {
            self.states.remove(&id);
            return self.event(writer, (STATE, Some(id)), at, kind::GONE, &[]);
        };
        let custom = writer.blob(&mut &now.custom[..], now.custom.len() as u64)?;
        let owner_name = writer.blob(&mut &now.owner_name[..], now.owner_name.len() as u64)?;
        let (owner, of) = match &now.owner {
            Owner::Tag(Ok(tag)) => (0, Cell::Text(writer.text(tag)?)),
            Owner::Tag(Err(why)) => (1, Cell::Text(writer.text(why)?)),
            Owner::Unlinked(link) => (2, Cell::Unsigned(u64::from(*link))),
        };
        let cells = [
            Cell::Text(custom),
            Cell::Text(owner_name),
            Cell::Unsigned(owner),
            of,
        ];
        self.event(writer, (STATE, Some(id)), at, kind::STATE, &cells)?;
        self.states.insert(id, now);
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)
    }

    /// What the emit goes on with after a cut: the last hour kept, every province's and state's
    /// inputs, the tables' holds and the open walked run.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let bytes = |out: &mut Vec<u8>, bytes: &[u8]| {
            out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            out.extend_from_slice(bytes);
        };
        let text = |out: &mut Vec<u8>, text: Option<&str>| match text {
            Some(text) => {
                out.push(1);
                bytes(out, text.as_bytes());
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
        match &self.tables {
            Some(tables) => {
                out.push(1);
                out.push(u8::from(tables.renames_complete));
                bytes(out, tables.renames_gap.as_bytes());
                text(out, tables.states_unrecorded.as_deref());
                bytes(out, tables.states_gap.as_bytes());
                out.push(u8::from(tables.reserved.is_ok()));
                bytes(out, tables.reserved.as_ref().unwrap_or_else(|why| why).as_bytes());
            }
            None => out.push(0),
        }
        let mut provinces: Vec<_> = self.provinces.iter().collect();
        provinces.sort_unstable_by_key(|(id, _)| **id);
        out.extend_from_slice(&(provinces.len() as u64).to_le_bytes());
        for (id, province) in provinces {
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&province.state.to_le_bytes());
            match &province.rename {
                Some(rename) => {
                    out.push(1);
                    bytes(out, rename);
                }
                None => out.push(0),
            }
        }
        let mut states: Vec<_> = self.states.iter().collect();
        states.sort_unstable_by_key(|(id, _)| **id);
        out.extend_from_slice(&(states.len() as u64).to_le_bytes());
        for (id, state) in states {
            out.extend_from_slice(&id.to_le_bytes());
            bytes(out, &state.custom);
            bytes(out, &state.owner_name);
            match &state.owner {
                Owner::Tag(Ok(tag)) => {
                    out.push(0);
                    bytes(out, tag.as_bytes());
                }
                Owner::Tag(Err(why)) => {
                    out.push(1);
                    bytes(out, why.as_bytes());
                }
                Owner::Unlinked(link) => {
                    out.push(2);
                    out.extend_from_slice(&link.to_le_bytes());
                }
            }
        }
        self.carry.save(out)
    }

    fn load(&mut self, saved: &[u8]) -> io::Result<()> {
        let mut d = Decode::new(saved);
        fn bytes(d: &mut Decode<'_>) -> io::Result<Vec<u8>> {
            let length = d.u32()? as usize;
            Ok(d.bytes(length)?.to_vec())
        }
        fn text(d: &mut Decode<'_>) -> io::Result<String> {
            String::from_utf8(bytes(d)?).map_err(|_| invalid())
        }
        self.last = d.flag()?.then(|| d.i32()).transpose()?;
        self.tables = if d.flag()? {
            let renames_complete = d.flag()?;
            let renames_gap = text(&mut d)?;
            let states_unrecorded = d.flag()?.then(|| text(&mut d)).transpose()?;
            let states_gap = text(&mut d)?;
            let known = d.flag()?;
            let reserved = text(&mut d)?;
            Some(Tables {
                renames_complete,
                renames_gap,
                states_unrecorded,
                states_gap,
                reserved: if known { Ok(reserved) } else { Err(reserved) },
            })
        } else {
            None
        };
        for _ in 0..d.u64()? {
            let id = d.u32()?;
            let state = d.u32()?;
            let rename = d.flag()?
                .then(|| bytes(&mut d).map(Vec::into_boxed_slice))
                .transpose()?;
            self.provinces.insert(id, Province { state, rename });
        }
        for _ in 0..d.u64()? {
            let id = d.u32()?;
            let custom = bytes(&mut d)?;
            let owner_name = bytes(&mut d)?;
            let owner = match d.u8()? {
                0 => Owner::Tag(Ok(text(&mut d)?)),
                1 => Owner::Tag(Err(text(&mut d)?)),
                2 => Owner::Unlinked(d.u32()?),
                _ => return Err(invalid()),
            };
            self.states.insert(
                id,
                StateInputs {
                    custom,
                    owner_name,
                    owner,
                },
            );
        }
        self.carry = Carry::load(&saved[d.position()..])?;
        Ok(())
    }
}
