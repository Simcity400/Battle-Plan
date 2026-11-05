//! Air statistics' emit on one branch: the hours its segments' operators observed
//! ([`super::segment`]), in order. It holds the read-time bindings each accepted read hands
//! on, to decide the hours a segment hands on before its first read, and the units' values the
//! operators last observed; and keeps the branch's one observation: whether it is open, and the
//! read dating each known hour. It writes only what an hour moved.
//!
//! The publication ([`SCHEMA`]): every unit (`TAG@slot|region`) is an actor, numbered in the
//! order the observation first held it, whose measure `f` holds field `f` of its
//! [`super::segment::FIELDS`]; [`FAMILY`]'s [`READ`] and [`KNOWN`] keys hold each known hour's
//! supplying read and 1; every key is bound to one domain, known at each hour walked; scalar
//! [`REFUSALS`] holds the refused observations by reason, as JSON.

use super::segment::{self, ALL, FIELDS, Segments, Values, Verdict};
use dashboard_base::prepared::series::{compress, decompress};
use dashboard_keyed::segment::Hour;
use dashboard_series::{
    Cell, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    family::{Actors, Directory, Place},
};
use replay_data::codec::Decoder;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// The actor of the family's own keys, [`READ`] and [`KNOWN`], by measure.
pub const FAMILY: u64 = u64::MAX;
/// The family's own measures: each known hour's supplying read, and 1 at each known hour.
pub const READ: u64 = 0;
pub const KNOWN: u64 = 1;
/// The family scalar holding the refused observations.
pub const REFUSALS: u64 = 0;
/// Every key's evidence: each hour walked is known.
const DOMAIN: u64 = 0;
const NONE: u32 = u32::MAX;

fn invalid(what: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what)
}

fn local(ids: &[u32], at: u32) -> io::Result<u32> {
    ids.get(at as usize)
        .copied()
        .ok_or_else(|| invalid("An air-statistics operator named what it never numbered"))
}

/// A unit the operators named: its country name and region, its first row's key and values as
/// last observed, its place among those held now, and its first key once published.
struct Unit {
    name: u32,
    region: u32,
    position: u64,
    values: [i64; segment::FIELDS],
    at: u32,
    keys: u32,
    seen: u32,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    names: Vec<Box<str>>,
    /// Each unit: its name, region, first row and its values where held now.
    units: Vec<(u32, u32, u64, Option<[i64; segment::FIELDS]>)>,
    bindings: Option<Vec<(u32, u32)>>,
    read_hour: Option<i32>,
    refusals: BTreeMap<String, u64>,
    open: bool,
    last_read: Option<i32>,
    refused_read: Option<i32>,
    last: Option<i32>,
}

/// The family's emit on one branch.
pub struct Emit {
    names: Vec<Box<str>>,
    name_ids: HashMap<Box<str>, u32>,
    units: Vec<Unit>,
    unit_ids: HashMap<(u32, u32), u32>,
    /// The segment's names and units, by the number its operator gave them.
    local_names: Vec<u32>,
    local_units: Vec<u32>,
    /// The units held now, in no order.
    present: Vec<u32>,
    /// The bindings of the last accepted read, by raw reference, and its hour.
    bindings: Option<HashMap<u32, u32>>,
    read_hour: Option<i32>,
    refusals: BTreeMap<String, u64>,
    /// Whether refusals changed since the last cut.
    noted: bool,
    /// Whether the last hour's observation was known, and the read dating it.
    open: bool,
    last_read: Option<i32>,
    /// The read held when consecutive refused hours closed the observation.
    refused_read: Option<i32>,
    actors: Actors,
    /// Each published unit's keys, by field from its first.
    handles: Vec<KeyHandle>,
    /// READ's and KNOWN's keys.
    own: [Option<KeyHandle>; 2],
    carry: Carry,
    /// The first hour walked the domain has not appended, and the last hour walked.
    first: Option<i32>,
    last: Option<i32>,
    generation: u32,
    /// What the last values applied moved: fields of units held before, units new, units gone.
    moved: Vec<(u32, u8)>,
    added: Vec<u32>,
    dropped: Vec<u32>,
}

impl Default for Emit {
    fn default() -> Self {
        Self::new()
    }
}

impl Emit {
    pub fn new() -> Self {
        Self {
            names: Vec::new(),
            name_ids: HashMap::new(),
            units: Vec::new(),
            unit_ids: HashMap::new(),
            local_names: Vec::new(),
            local_units: Vec::new(),
            present: Vec::new(),
            bindings: None,
            read_hour: None,
            refusals: BTreeMap::new(),
            noted: false,
            open: false,
            last_read: None,
            refused_read: None,
            actors: Actors::new(SCHEMA),
            handles: Vec::new(),
            own: [None; 2],
            carry: Carry::default(),
            first: None,
            last: None,
            generation: 0,
            moved: Vec::new(),
            added: Vec::new(),
            dropped: Vec::new(),
        }
    }

    fn name(&mut self, name: Box<str>) -> u32 {
        if let Some(&id) = self.name_ids.get(&name) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(name.clone());
        self.name_ids.insert(name, id);
        id
    }

    fn unit(&mut self, name: u32, region: u32) -> u32 {
        let next = self.units.len() as u32;
        let id = *self.unit_ids.entry((name, region)).or_insert(next);
        if id == next {
            self.units.push(Unit {
                name,
                region,
                position: 0,
                values: [0; FIELDS],
                at: NONE,
                keys: NONE,
                seen: 0,
            });
        }
        id
    }

    /// One hour, after the last one given.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: Hour<Segments>) -> io::Result<()> {
        let at = hour.hour;
        let continuous = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last
            && !continuous
        {
            self.carry.close(writer, last)?;
            self.known(writer, last)?;
        }
        match hour.read {
            // Nothing it reads moved: the observation holds, after refused hours or a gap
            // refused.
            None if self.open && continuous => {
                if hour.dated {
                    self.last_read = Some(at);
                    self.put_own(writer, READ, at, Some(i128::from(at)), false)?;
                }
            }
            None => self.refused(writer, at, continuous)?,
            Some(facts) => self.observe(writer, at, hour.dated, continuous, facts)?,
        }
        self.first.get_or_insert(at);
        self.last = Some(at);
        Ok(())
    }

    fn observe(
        &mut self,
        writer: &mut Writer<'_>,
        at: i32,
        read: bool,
        continuous: bool,
        facts: segment::Facts,
    ) -> io::Result<()> {
        let segment::Facts {
            fresh,
            verdict,
            names,
            units,
            values,
            bindings,
        } = facts;
        if fresh {
            self.local_names.clear();
            self.local_units.clear();
        }
        for name in names {
            let id = self.name(name);
            self.local_names.push(id);
        }
        for (name, region) in units {
            let name = local(&self.local_names, name)?;
            let id = self.unit(name, region);
            self.local_units.push(id);
        }
        let previous = if read {
            self.read_hour = None;
            self.bindings.take()
        } else {
            None
        };
        let verdict = match verdict {
            Verdict::Accepted => Ok(()),
            Verdict::Refused(why) => Err(String::from(why)),
            Verdict::Held(names) => self.held(&names)?,
        };
        if read && verdict.is_ok() {
            let bindings =
                bindings.ok_or_else(|| invalid("An accepted air-statistics read bound nothing"))?;
            let mut bound = if bindings.replace {
                HashMap::with_capacity(bindings.set.len())
            } else {
                previous.unwrap_or_default()
            };
            for raw in bindings.removed {
                bound.remove(&raw);
            }
            for (raw, name) in bindings.set {
                bound.insert(raw, local(&self.local_names, name)?);
            }
            self.bindings = Some(bound);
            self.read_hour = Some(at);
        }
        let starts = !self.open || !continuous;
        let why = match verdict {
            Ok(()) => None,
            Err(why) => Some(why),
        };
        if let Some(why) = why {
            *self.refusals.entry(why).or_default() += 1;
            self.noted = true;
            self.refused(writer, at, continuous)?;
            if let Some(values) = &values {
                self.apply(values)?;
            }
            return Ok(());
        }
        let values =
            values.ok_or_else(|| invalid("An accepted air-statistics hour observed no values"))?;
        if read {
            self.last_read = Some(at);
        } else if starts {
            // The values carry an earlier read: the one held before refused hours just ended,
            // when the bindings were frozen there; otherwise undated.
            self.last_read = self
                .refused_read
                .filter(|&held| continuous && self.read_hour == Some(held));
        }
        self.refused_read = None;
        let open = self.open;
        self.apply(&values)?;
        if starts {
            if open {
                for at_drop in 0..self.dropped.len() {
                    self.end(writer, self.dropped[at_drop], at)?;
                }
            }
            let mut fresh: Vec<u32> = self
                .present
                .iter()
                .copied()
                .filter(|&unit| self.units[unit as usize].keys == NONE)
                .collect();
            self.publish_new(writer, &mut fresh, at)?;
            for index in 0..self.present.len() {
                self.restate(writer, self.present[index], at, true)?;
            }
            self.put_own(writer, KNOWN, at, Some(1), true)?;
        } else {
            let mut added = std::mem::take(&mut self.added);
            self.publish_new(writer, &mut added, at)?;
            for &unit in &added {
                self.restate(writer, unit, at, false)?;
            }
            self.added = added;
            for index in 0..self.moved.len() {
                let (unit, field) = self.moved[index];
                let unit = &self.units[unit as usize];
                let value = unit.values[usize::from(field)];
                let keys = unit.keys;
                let handle = self.handle(keys, usize::from(field))?;
                self.put(writer, handle, at, Some(i128::from(value)), false)?;
            }
            for index in 0..self.dropped.len() {
                self.end(writer, self.dropped[index], at)?;
            }
        }
        if read || starts {
            let dated = self.last_read.map(i128::from);
            self.put_own(writer, READ, at, dated, starts)?;
        }
        self.open = true;
        Ok(())
    }

    /// Whether each row country's name now is the one its read bound.
    fn held(&self, names: &[(u32, Option<u32>)]) -> io::Result<Result<(), String>> {
        let Some(bound) = &self.bindings else {
            return Ok(Err(segment::UNBOUND.into()));
        };
        for &(raw, name) in names {
            let now = name.map(|name| local(&self.local_names, name)).transpose()?;
            if now.is_none() || bound.get(&raw).copied() != now {
                return Ok(Err(segment::mismatched(raw)));
            }
        }
        Ok(Ok(()))
    }

    /// The units' values an hour observed, into those held: what it moved, came and went.
    fn apply(&mut self, values: &Values) -> io::Result<()> {
        self.moved.clear();
        self.added.clear();
        self.dropped.clear();
        self.generation += 1;
        let generation = self.generation;
        for &(unit, position) in &values.placed {
            let unit = local(&self.local_units, unit)?;
            self.units[unit as usize].position = position;
        }
        let mut packed = values.packed.iter().copied();
        let missing = || invalid("An air-statistics unit's values are short");
        for &(unit, mask) in &values.changed {
            let id = local(&self.local_units, unit)?;
            let held = &mut self.units[id as usize];
            held.seen = generation;
            if held.at == NONE {
                if mask != ALL {
                    return Err(invalid("A new air-statistics unit misses values"));
                }
                for value in &mut held.values {
                    *value = packed.next().ok_or_else(missing)?;
                }
                held.at = self.present.len() as u32;
                self.present.push(id);
                self.added.push(id);
                continue;
            }
            for field in 0..FIELDS {
                if mask & (1 << field) == 0 {
                    continue;
                }
                let value = packed.next().ok_or_else(missing)?;
                if held.values[field] != value {
                    held.values[field] = value;
                    self.moved.push((id, field as u8));
                }
            }
        }
        if packed.next().is_some() {
            return Err(invalid("An air-statistics unit's values are long"));
        }
        let gone: Vec<u32> = if values.whole {
            self.present
                .iter()
                .copied()
                .filter(|&unit| self.units[unit as usize].seen != generation)
                .collect()
        } else {
            values
                .removed
                .iter()
                .map(|&unit| local(&self.local_units, unit))
                .collect::<io::Result<_>>()?
        };
        for unit in gone {
            let at = self.units[unit as usize].at;
            if at == NONE {
                return Err(invalid("An air-statistics unit gone was never held"));
            }
            self.present.swap_remove(at as usize);
            if let Some(&moved) = self.present.get(at as usize) {
                self.units[moved as usize].at = at;
            }
            self.units[unit as usize].at = NONE;
            self.dropped.push(unit);
        }
        Ok(())
    }

    /// An hour whose inputs are not known: every value becomes unknown.
    fn refused(&mut self, writer: &mut Writer<'_>, at: i32, continuous: bool) -> io::Result<()> {
        if self.open {
            self.put_own(writer, READ, at, None, false)?;
            self.put_own(writer, KNOWN, at, None, false)?;
            for index in 0..self.present.len() {
                self.end(writer, self.present[index], at)?;
            }
        }
        // Keep the read only across consecutive refused hours, never a gap.
        self.refused_read = if continuous {
            self.last_read.or(self.refused_read)
        } else {
            None
        };
        self.open = false;
        self.last_read = None;
        Ok(())
    }

    /// Numbers `units`, in their first rows' order, as the observation first holds them.
    fn publish_new(
        &mut self,
        writer: &mut Writer<'_>,
        units: &mut [u32],
        at: i32,
    ) -> io::Result<()> {
        units.sort_unstable_by_key(|&unit| self.units[unit as usize].position);
        for &unit in units.iter() {
            self.number(writer, unit, at)?;
        }
        Ok(())
    }

    fn number(&mut self, writer: &mut Writer<'_>, unit: u32, at: i32) -> io::Result<()> {
        let held = &self.units[unit as usize];
        if held.keys != NONE {
            return Ok(());
        }
        let name = format!("{}|{}", self.names[held.name as usize], held.region);
        let actor = self.actors.actor(writer, name.as_bytes(), at)?;
        let first = u32::try_from(self.handles.len())
            .map_err(|_| invalid("Air-statistics keys exceed their numbering"))?;
        for field in 0..FIELDS {
            let place = Place {
                actor,
                measure: field as u64,
                dimension: None,
            };
            let key = self
                .actors
                .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))?;
            self.handles.push(key);
        }
        self.units[unit as usize].keys = first;
        Ok(())
    }

    fn handle(&self, keys: u32, field: usize) -> io::Result<KeyHandle> {
        if keys == NONE {
            return Err(invalid("An air-statistics unit held is not published"));
        }
        Ok(self.handles[keys as usize + field])
    }

    /// Every value of `unit` from `at` on.
    fn restate(
        &mut self,
        writer: &mut Writer<'_>,
        unit: u32,
        at: i32,
        starts: bool,
    ) -> io::Result<()> {
        if self.units[unit as usize].keys == NONE {
            self.number(writer, unit, at)?;
        }
        let held = &self.units[unit as usize];
        let (keys, values) = (held.keys, held.values);
        for (field, value) in values.into_iter().enumerate() {
            let handle = self.handle(keys, field)?;
            self.put(writer, handle, at, Some(i128::from(value)), starts)?;
        }
        Ok(())
    }

    /// No value of `unit` from `at` on.
    fn end(&mut self, writer: &mut Writer<'_>, unit: u32, at: i32) -> io::Result<()> {
        let keys = self.units[unit as usize].keys;
        if keys == NONE {
            return Ok(());
        }
        for field in 0..FIELDS {
            let handle = self.handle(keys, field)?;
            self.put(writer, handle, at, None, false)?;
        }
        Ok(())
    }

    /// `handle`'s value from `at` on, a new observation's when `starts`.
    fn put(
        &mut self,
        writer: &mut Writer<'_>,
        handle: KeyHandle,
        at: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()> {
        // A run breaks where its observation starts or the hour before held none.
        let break_before = starts || self.carry.held(handle).is_none();
        self.carry.value(writer, handle, at, value, break_before)?;
        Ok(())
    }

    fn put_own(
        &mut self,
        writer: &mut Writer<'_>,
        measure: u64,
        at: i32,
        value: Option<i128>,
        starts: bool,
    ) -> io::Result<()> {
        for own in [READ, KNOWN] {
            if self.own[own as usize].is_none() {
                let place = Place {
                    actor: FAMILY,
                    measure: own,
                    dimension: None,
                };
                let key = self
                    .actors
                    .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))?;
                self.own[own as usize] = Some(key);
            }
        }
        let handle = self.own[measure as usize].expect("the family's own keys are made");
        self.put(writer, handle, at, value, starts)
    }

    /// The hours walked since the domain's last row, through `last`, as known.
    fn known(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
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
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, and the refusals where they changed.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        self.carry.cut(writer, last)?;
        self.known(writer, last)?;
        if self.noted {
            let json = serde_json::to_vec(&self.refusals)?;
            let text = writer.blob(&mut &json[..], json.len() as u64)?;
            writer.catalogue(REFUSALS, Cell::Text(text))?;
            self.noted = false;
        }
        Ok(())
    }

    /// What it goes on with after a cut: its observation and open runs.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            names: self.names.clone(),
            units: self
                .units
                .iter()
                .map(|unit| {
                    let values = (unit.at != NONE).then_some(unit.values);
                    (unit.name, unit.region, unit.position, values)
                })
                .collect(),
            bindings: self
                .bindings
                .as_ref()
                .map(|bound| bound.iter().map(|(&raw, &name)| (raw, name)).collect()),
            read_hour: self.read_hour,
            refusals: self.refusals.clone(),
            open: self.open,
            last_read: self.last_read,
            refused_read: self.refused_read,
            last: self.last,
        };
        let saved = compress(&serde_json::to_vec(&saved)?, 3)?;
        out.extend_from_slice(&(saved.len() as u64).to_le_bytes());
        out.extend_from_slice(&saved);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] wrote, going on from the publication `directory` reads at a
    /// segment's first hour.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut input = Decoder::new(bytes);
        let length = input.count64(1)?;
        let saved: Saved = serde_json::from_slice(&decompress(input.take(length)?)?)?;
        let mut emit = Self::new();
        for name in saved.names {
            emit.name(name);
        }
        for (name, region, position, values) in saved.units {
            if name as usize >= emit.names.len() {
                return Err(invalid("A saved air-statistics unit names no country"));
            }
            let id = emit.unit(name, region);
            let unit = &mut emit.units[id as usize];
            unit.position = position;
            if let Some(values) = values {
                unit.values = values;
                unit.at = emit.present.len() as u32;
                emit.present.push(id);
            }
            let spelled = format!("{}|{region}", emit.names[name as usize]);
            if let Some(actor) = directory.actor(spelled.as_bytes()) {
                emit.units[id as usize].keys = emit.handles.len() as u32;
                for field in 0..FIELDS {
                    let key = directory
                        .key(actor, field as u64, None)
                        .ok_or_else(|| invalid("A published air-statistics unit misses a key"))?;
                    emit.handles.push(key);
                }
            }
        }
        emit.bindings = saved.bindings.map(|bound| bound.into_iter().collect());
        emit.read_hour = saved.read_hour;
        emit.refusals = saved.refusals;
        emit.open = saved.open;
        emit.last_read = saved.last_read;
        emit.refused_read = saved.refused_read;
        emit.last = saved.last;
        emit.own = [
            directory.key(FAMILY, READ, None),
            directory.key(FAMILY, KNOWN, None),
        ];
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.carry = Carry::load(input.rest())?;
        Ok(emit)
    }
}
