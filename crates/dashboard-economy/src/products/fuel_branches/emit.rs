//! Fuel by branch's ordered emit: holders are actors named by their canonical slot.

use super::facts::{self, Cell, Values};
use dashboard_series::{
    Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
    hash::FastMap,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub const SCHEMA: u64 = 1;
pub const READ: u64 = 0;
pub const REQUESTED: u64 = 1;
pub const RECEIVED: u64 = 2;
pub const PRIORITY: u64 = 3;
pub const KNOWN: u64 = 0;
pub const FAMILY: u64 = u64::MAX;
/// Scalars: `TAGS + actor` holds `[[hour, tag]…]`, `REASONS + id` holds `[reason, hours]`.
pub const TAGS: u64 = 1 << 20;
pub const REASONS: u64 = 1 << 30;
const DOMAIN: u64 = 0;
/// Counted once per unresolved row at each observed hour, as the census counts it.
pub const UNIDENTIFIED: &str = "a fuel consumer's country has no identity";

fn invalid() -> io::Error {
    dashboard_base::invalid("Saved fuel branches emit is not valid")
}

#[derive(Clone, Default)]
struct Holder {
    actor: u64,
    read_key: KeyHandle,
    cells: u32,
    read: Option<i32>,
    tags: Vec<(i32, Box<str>)>,
}

pub struct Emit {
    actors: Actors,
    carry: Carry,
    holders: FastMap<u32, Holder>,
    keys: FastMap<Cell, [KeyHandle; 3]>,
    cells: BTreeMap<Cell, Values>,
    known: Option<KeyHandle>,
    unidentified: u32,
    reasons: Vec<(Box<str>, u64)>,
    numbered: FastMap<Box<str>, u64>,
    refused: BTreeSet<u64>,
    renamed: BTreeSet<u32>,
    fresh: Vec<u32>,
    emptied: Vec<u32>,
    last: Option<i32>,
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
            holders: FastMap::default(),
            keys: FastMap::default(),
            cells: BTreeMap::new(),
            known: None,
            unidentified: 0,
            reasons: Vec::new(),
            numbered: FastMap::default(),
            refused: BTreeSet::new(),
            renamed: BTreeSet::new(),
            fresh: Vec::new(),
            emptied: Vec::new(),
            last: None,
            first: None,
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))
    }

    fn known(&mut self, writer: &mut Writer<'_>) -> io::Result<KeyHandle> {
        if let Some(key) = self.known {
            return Ok(key);
        }
        let key = self.key(
            writer,
            Place {
                actor: FAMILY,
                measure: KNOWN,
                dimension: None,
            },
        )?;
        self.known = Some(key);
        Ok(key)
    }

    fn holder(&mut self, writer: &mut Writer<'_>, slot: u32, hour: i32) -> io::Result<&mut Holder> {
        if !self.holders.contains_key(&slot) {
            let actor = self.actors.actor(writer, slot.to_string().as_bytes(), hour)?;
            let read_key = self.key(
                writer,
                Place {
                    actor,
                    measure: READ,
                    dimension: None,
                },
            )?;
            self.holders.insert(
                slot,
                Holder {
                    actor,
                    read_key,
                    ..Holder::default()
                },
            );
        }
        Ok(self.holders.get_mut(&slot).expect("the holder just held"))
    }

    fn cell_keys(&mut self, writer: &mut Writer<'_>, cell: Cell, hour: i32) -> io::Result<[KeyHandle; 3]> {
        if let Some(&keys) = self.keys.get(&cell) {
            return Ok(keys);
        }
        let actor = self.holder(writer, cell.0, hour)?.actor;
        let mut keys = [0; 3];
        for (key, measure) in keys.iter_mut().zip([REQUESTED, RECEIVED, PRIORITY]) {
            *key = self.key(
                writer,
                Place {
                    actor,
                    measure,
                    dimension: Some(u64::from(cell.1)),
                },
            )?;
        }
        self.keys.insert(cell, keys);
        Ok(keys)
    }

    fn refuse(&mut self, reason: &str, times: u64) {
        if times == 0 {
            return;
        }
        let id = match self.numbered.get(reason) {
            Some(&id) => id,
            None => {
                let id = self.reasons.len() as u64;
                self.reasons.push((reason.into(), 0));
                self.numbered.insert(reason.into(), id);
                id
            }
        };
        self.reasons[id as usize].1 += times;
        self.refused.insert(id);
    }

    fn set(&mut self, writer: &mut Writer<'_>, at: i32, cell: Cell, values: Values) -> io::Result<()> {
        let keys = self.cell_keys(writer, cell, at)?;
        let held = self.cells.insert(cell, values);
        if held.is_none() {
            let holder = self.holder(writer, cell.0, at)?;
            holder.cells += 1;
            if holder.cells == 1 {
                self.fresh.push(cell.0);
            }
        }
        for (part, key) in keys.into_iter().enumerate() {
            if held.map(|held| held[part]) != Some(values[part]) {
                self.carry
                    .value(writer, key, at, Some(i128::from(values[part])), false)?;
            }
        }
        Ok(())
    }

    fn end(&mut self, writer: &mut Writer<'_>, at: i32, cell: Cell) -> io::Result<()> {
        if self.cells.remove(&cell).is_none() {
            return Ok(());
        }
        for key in self.keys[&cell] {
            self.carry.value(writer, key, at, None, false)?;
        }
        let holder = self
            .holders
            .get_mut(&cell.0)
            .ok_or_else(|| io::Error::other("A held fuel consumer has no holder"))?;
        holder.cells -= 1;
        if holder.cells == 0 {
            self.emptied.push(cell.0);
        }
        Ok(())
    }

    fn settle(&mut self, writer: &mut Writer<'_>, at: i32) -> io::Result<()> {
        for slot in std::mem::take(&mut self.emptied) {
            if let Some(holder) = self.holders.get_mut(&slot)
                && holder.cells == 0
                && holder.read.take().is_some()
            {
                self.carry.value(writer, holder.read_key, at, None, false)?;
            }
        }
        Ok(())
    }

    fn apply(&mut self, writer: &mut Writer<'_>, at: i32, facts: facts::Facts) -> io::Result<()> {
        let facts::Facts {
            whole,
            cells,
            unidentified,
            tags,
        } = facts;
        if whole {
            let named: BTreeSet<Cell> = cells.iter().map(|&(cell, _)| cell).collect();
            let ended: Vec<Cell> = self
                .cells
                .keys()
                .filter(|cell| !named.contains(cell))
                .copied()
                .collect();
            for cell in ended {
                self.end(writer, at, cell)?;
            }
        }
        for (cell, values) in cells {
            match values {
                Some(values) => self.set(writer, at, cell, values)?,
                None => self.end(writer, at, cell)?,
            }
        }
        self.unidentified = unidentified;
        for (slot, tag) in tags {
            let Some(tag) = tag else {
                continue;
            };
            let Some(holder) = self.holders.get_mut(&slot).filter(|holder| holder.cells > 0)
            else {
                continue;
            };
            if holder.tags.last().is_none_or(|(_, last)| **last != *tag) {
                holder.tags.push((at, Box::from(&*tag)));
                self.renamed.insert(slot);
            }
        }
        self.settle(writer, at)
    }

    fn clear(&mut self, writer: &mut Writer<'_>, at: i32) -> io::Result<()> {
        let held: Vec<Cell> = self.cells.keys().copied().collect();
        for cell in held {
            self.end(writer, at, cell)?;
        }
        if let Some(key) = self.known {
            self.carry.value(writer, key, at, None, false)?;
        }
        self.unidentified = 0;
        self.fresh.clear();
        self.settle(writer, at)
    }

    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("fuel_branches.emit", None);
        let at = hour.hour;
        let gap = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => false,
            Some(last) if at <= last => {
                return Err(dashboard_base::invalid(
                    "A fuel branches hour does not follow the hour before",
                ));
            }
            Some(last) => {
                self.carry.close(writer, last)?;
                true
            }
            None => true,
        };
        self.last = Some(at);
        self.first.get_or_insert(at);
        if !(hour.observed || gap) {
            // A segment's first hour hands every cell on, though the census does not observe it.
            if let Some(facts) = hour.facts {
                self.apply(writer, at, *facts)?;
            }
            self.fresh.clear();
            return Ok(());
        }
        // The row census dates every consumer anew after an hour not walked.
        let read = hour.read || gap;
        if let Some(reason) = hour.refusal {
            self.refuse(&reason, 1);
            return self.clear(writer, at);
        }
        if let Some(facts) = hour.facts {
            self.apply(writer, at, *facts)?;
        }
        if gap {
            // Every run ended at the hour not walked: what is held starts again.
            let held: Vec<(Cell, Values)> = self.cells.iter().map(|(&cell, &values)| (cell, values)).collect();
            for (cell, values) in held {
                for (part, key) in self.keys[&cell].into_iter().enumerate() {
                    self.carry
                        .value(writer, key, at, Some(i128::from(values[part])), false)?;
                }
            }
        }
        let fresh = std::mem::take(&mut self.fresh);
        if read {
            for holder in self.holders.values_mut().filter(|holder| holder.cells > 0) {
                holder.read = Some(at);
                self.carry
                    .value(writer, holder.read_key, at, Some(i128::from(at)), false)?;
            }
        } else {
            for slot in fresh {
                if let Some(holder) = self.holders.get_mut(&slot)
                    && holder.cells > 0
                    && holder.read.is_none()
                {
                    holder.read = Some(at);
                    self.carry
                        .value(writer, holder.read_key, at, Some(i128::from(at)), false)?;
                }
            }
        }
        let known = self.known(writer)?;
        let complete = (self.unidentified == 0).then_some(1);
        self.carry.value(writer, known, at, complete, false)?;
        self.refuse(UNIDENTIFIED, u64::from(self.unidentified));
        Ok(())
    }

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
        for id in std::mem::take(&mut self.refused) {
            let (reason, hours) = &self.reasons[id as usize];
            let scalar = Scalar::List(vec![
                Scalar::Text(reason.as_bytes().into()),
                Scalar::Unsigned(*hours),
            ]);
            let cell = family::cell(writer, &scalar)?;
            writer.catalogue(scalar_id(REASONS, id)?, cell)?;
        }
        for slot in std::mem::take(&mut self.renamed) {
            let holder = &self.holders[&slot];
            let scalar = Scalar::List(
                holder
                    .tags
                    .iter()
                    .map(|(hour, tag)| {
                        Scalar::List(vec![
                            Scalar::Integer(i128::from(*hour)),
                            Scalar::Text(tag.as_bytes().into()),
                        ])
                    })
                    .collect(),
            );
            let cell = family::cell(writer, &scalar)?;
            writer.catalogue(scalar_id(TAGS, holder.actor)?, cell)?;
        }
        Ok(())
    }

    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            last: self.last,
            unidentified: self.unidentified,
            cells: self
                .cells
                .iter()
                .map(|(&cell, &values)| (cell, values))
                .collect(),
            reads: self
                .holders
                .iter()
                .filter_map(|(&slot, holder)| Some((slot, holder.read?)))
                .collect(),
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        self.carry.save(out)
    }

    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.known = directory.key(FAMILY, KNOWN, None);
        for (actor, named) in directory.actors.iter().enumerate() {
            let actor = actor as u64;
            let slot: u32 = std::str::from_utf8(&named.name)
                .ok()
                .and_then(|name| name.parse().ok())
                .ok_or_else(invalid)?;
            let read_key = directory.key(actor, READ, None).ok_or_else(invalid)?;
            let tags = query_tags(directory, actor)?
                .into_iter()
                .map(|(hour, tag)| (hour, tag.into_boxed_str()))
                .collect();
            emit.holders.insert(
                slot,
                Holder {
                    actor,
                    read_key,
                    cells: 0,
                    read: None,
                    tags,
                },
            );
            let dimensions = [REQUESTED, RECEIVED, PRIORITY].map(|measure| directory.dimensions(actor, measure));
            if dimensions[1].len() != dimensions[0].len() || dimensions[2].len() != dimensions[0].len() {
                return Err(invalid());
            }
            for at in 0..dimensions[0].len() {
                let index = dimensions[0][at].0;
                if dimensions[1][at].0 != index || dimensions[2][at].0 != index {
                    return Err(invalid());
                }
                let index = u32::try_from(index).map_err(|_| invalid())?;
                emit.keys.insert(
                    (slot, index),
                    [dimensions[0][at].1, dimensions[1][at].1, dimensions[2][at].1],
                );
            }
        }
        let mut id = 0;
        while let Some(scalar) = directory.scalar(scalar_id(REASONS, id)?) {
            let (reason, hours) = reason_of(scalar).ok_or_else(invalid)?;
            emit.numbered.insert(reason.as_str().into(), id);
            emit.reasons.push((reason.into_boxed_str(), hours));
            id += 1;
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
        emit.unidentified = saved.unidentified;
        for (cell, values) in saved.cells {
            if !emit.keys.contains_key(&cell) {
                return Err(invalid());
            }
            emit.holders.get_mut(&cell.0).ok_or_else(invalid)?.cells += 1;
            emit.cells.insert(cell, values);
        }
        for (slot, read) in saved.reads {
            emit.holders.get_mut(&slot).ok_or_else(invalid)?.read = Some(read);
        }
        Ok(emit)
    }
}

fn scalar_id(base: u64, id: u64) -> io::Result<u64> {
    let limit = if base == TAGS { REASONS } else { family::ACTORS };
    base.checked_add(id)
        .filter(|&at| at < limit)
        .ok_or_else(|| io::Error::other("Fuel branches scalar ids exhausted"))
}

pub fn query_tags(directory: &Directory, actor: u64) -> io::Result<Vec<(i32, String)>> {
    let Some(scalar) = directory.scalar(scalar_id(TAGS, actor)?) else {
        return Ok(Vec::new());
    };
    scalar
        .list()
        .iter()
        .map(|pair| match pair.list() {
            [hour, tag] => Some((
                i32::try_from(hour.integer()?).ok()?,
                String::from_utf8(tag.text()?.to_vec()).ok()?,
            )),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| dashboard_base::invalid("Prepared fuel branch tags are not valid"))
}

pub fn reason_of(scalar: &Scalar) -> Option<(String, u64)> {
    match scalar.list() {
        [reason, hours] => Some((
            String::from_utf8(reason.text()?.to_vec()).ok()?,
            u64::try_from(hours.integer()?).ok()?,
        )),
        _ => None,
    }
}

pub fn reasons(directory: &Directory) -> io::Result<Vec<(String, u64)>> {
    let mut reasons = Vec::new();
    let mut id = 0;
    while let Some(scalar) = directory.scalar(scalar_id(REASONS, id)?) {
        reasons.push(
            reason_of(scalar)
                .ok_or_else(|| dashboard_base::invalid("Prepared fuel branch refusal is not valid"))?,
        );
        id += 1;
    }
    Ok(reasons)
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    unidentified: u32,
    cells: Vec<(Cell, Values)>,
    reads: Vec<(u32, i32)>,
}
