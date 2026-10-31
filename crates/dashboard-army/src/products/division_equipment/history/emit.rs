//! Division equipment's emit: each answered hour's changed values as runs on the family's
//! series publication, keyed by country, attribution and value, the operands they name
//! catalogued once by content. The hours a segment cannot answer alone are stepped on the
//! engine the segment before handed on ([`Projected`]).
//!
//! A value goes on while every hour gives it; an unknown hour ends every value, and the next
//! known hour, or one after an hour not walked, starts each again. A whole hour's values are
//! every value: any other ends.

use super::engine::{Change, Engine, Field, MEASURES, Operand, Output, PARTS, Value};
use super::facts::{Hour, Projected};
use super::{KeptReason, NamingOperands};
use crate::products::army_content::TemplateArtwork;
use dashboard_series::{
    Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{self, Actors, Directory, Place, Scalar},
};
use dashboard_text::{catalogue::Catalogue, template_facts::TemplateNaming};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::Arc,
};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// The actor of the family's own keys: whether the hour is known, and the hours of the pool
/// and stockpile reads every reply reads.
pub const FAMILY: u64 = u64::MAX;
pub const KNOWN: u64 = 0;
pub const POOL: u64 = 1;
pub const STOCK: u64 = 2;
/// A country's measures, by attribution: `side * SIDE + code`.
pub const SIDE: u64 = 32;
pub const STATE: u64 = 0;
pub const DIVISIONS: u64 = 1;
pub const SHORT: u64 = 2;
pub const UNKNOWN: u64 = 3;
pub const REFUSED: u64 = 4;
/// A type's measures, in [`MEASURES`] order, by kind.
pub const TYPE: u64 = 5;
/// A design's holding and naming operands, by kind and design ([`PAIRS`]).
pub const VARIANT: u64 = 12;
pub const DESIGN: u64 = 13;
/// A template's measures, in [`PARTS`] order, by template.
pub const TEMPLATE: u64 = 14;
/// A note's count, by note.
pub const NOTE: u64 = 25;
/// The stockpile read's naming operands of a design, by design.
pub const STOCK_DESIGN: u64 = 26;
/// Catalogue ranges, each of an operand by its number: an archetype's key (null for the
/// unclassified), a note, a template's naming and artwork, a design's naming operands, and
/// a kind and design pair.
const RANGE: u64 = 1 << 29;
pub const KINDS: u64 = RANGE;
pub const NOTES: u64 = 2 * RANGE;
pub const NAMINGS: u64 = 3 * RANGE;
pub const ARTWORKS: u64 = 4 * RANGE;
pub const DESIGNS: u64 = 5 * RANGE;
pub const PAIRS: u64 = 6 * RANGE;
/// Every value key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// A value's measure and dimension in its country's keys.
fn place(side: u8, field: Field, kind: u64, note: u64, pair: u64) -> (u64, Option<u64>) {
    let base = u64::from(side) * SIDE;
    let at = |list: usize| list as u64;
    match field {
        Field::State => (base + STATE, None),
        Field::Divisions => (base + DIVISIONS, None),
        Field::Short => (base + SHORT, None),
        Field::Unknown => (base + UNKNOWN, None),
        Field::Refused => (base + REFUSED, None),
        Field::Type(_, measure) => (
            base + TYPE
                + at(MEASURES
                    .iter()
                    .position(|held| *held == measure)
                    .unwrap_or(0)),
            Some(kind),
        ),
        Field::Variant(..) => (base + VARIANT, Some(pair)),
        Field::Design(..) => (base + DESIGN, Some(pair)),
        Field::Template(handle, part) => (
            base + TEMPLATE + at(PARTS.iter().position(|held| *held == part).unwrap_or(0)),
            Some(handle),
        ),
        Field::Note(_) => (base + NOTE, Some(note)),
        Field::StockDesign(handle) => (base + STOCK_DESIGN, Some(handle)),
    }
}

/// An engine's numbering, as the publication catalogues it: each operand's number in its
/// range, and each country's actor.
#[derive(Default)]
struct Numbers {
    operands: Vec<Option<u64>>,
    actors: Vec<Option<u64>>,
}

impl Numbers {
    fn operand(&self, id: u32) -> io::Result<u64> {
        self.operands
            .get(id as usize)
            .copied()
            .flatten()
            .ok_or_else(|| invalid("A division equipment value names an operand never given"))
    }

    fn actor(&self, tag: u32) -> io::Result<u64> {
        self.actors
            .get(tag as usize)
            .copied()
            .flatten()
            .ok_or_else(|| invalid("A division equipment value names a country never given"))
    }
}

/// The family's emit on one branch.
pub struct Emit {
    cat: Option<Arc<Catalogue>>,
    actors: Actors,
    carry: Carry,
    family: Option<[KeyHandle; 3]>,
    kinds: HashMap<Option<Arc<str>>, u64>,
    notes: HashMap<KeptReason, u64>,
    namings: BTreeMap<Result<TemplateNaming, KeptReason>, u64>,
    artworks: BTreeMap<Result<TemplateArtwork, KeptReason>, u64>,
    designs: HashMap<NamingOperands, u64>,
    pairs: HashMap<(u64, u64), u64>,
    /// The engine the segment before handed on, for the hours this one cannot answer, and
    /// its numbering; the numbering of the segment's own engine.
    handed: Option<(Box<Engine>, Numbers)>,
    current: Numbers,
    /// Every value key set and not ended, and the generation of the whole hour that last set
    /// it.
    open: HashMap<KeyHandle, u64>,
    generation: u64,
    last: Option<i32>,
    /// Whether the last hour was known.
    known: bool,
    /// The first hour since the last cut.
    first: Option<i32>,
}

impl Emit {
    /// An emit of a branch whose content is `cat`.
    pub fn new(cat: Option<Arc<Catalogue>>) -> Self {
        Self {
            cat,
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            family: None,
            kinds: HashMap::new(),
            notes: HashMap::new(),
            namings: BTreeMap::new(),
            artworks: BTreeMap::new(),
            designs: HashMap::new(),
            pairs: HashMap::new(),
            handed: None,
            current: Numbers::default(),
            open: HashMap::new(),
            generation: 0,
            last: None,
            known: false,
            first: None,
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))
    }

    /// The catalogued number of `operand`, catalogued at its first naming.
    fn catalogue(&mut self, writer: &mut Writer<'_>, operand: Operand) -> io::Result<u64> {
        fn json(value: &impl Serialize) -> io::Result<Scalar> {
            Ok(Scalar::Text(serde_json::to_vec(value)?.into_boxed_slice()))
        }
        let (range, number, cell) = match operand {
            Operand::Kind(key) => {
                if let Some(&number) = self.kinds.get(&key) {
                    return Ok(number);
                }
                let number = self.kinds.len() as u64;
                let cell = match &key {
                    Some(key) => Scalar::Text(key.as_bytes().into()),
                    None => Scalar::Null,
                };
                self.kinds.insert(key, number);
                (KINDS, number, cell)
            }
            Operand::Note(note) => {
                if let Some(&number) = self.notes.get(&*note) {
                    return Ok(number);
                }
                let number = self.notes.len() as u64;
                let cell = json(&*note)?;
                self.notes.insert((*note).clone(), number);
                (NOTES, number, cell)
            }
            Operand::Naming(naming) => {
                if let Some(&number) = self.namings.get(&*naming) {
                    return Ok(number);
                }
                let number = self.namings.len() as u64;
                let cell = json(&*naming)?;
                self.namings.insert((*naming).clone(), number);
                (NAMINGS, number, cell)
            }
            Operand::Artwork(artwork) => {
                if let Some(&number) = self.artworks.get(&*artwork) {
                    return Ok(number);
                }
                let number = self.artworks.len() as u64;
                let cell = json(&*artwork)?;
                self.artworks.insert((*artwork).clone(), number);
                (ARTWORKS, number, cell)
            }
            Operand::Design(raw) => {
                let operands = NamingOperands(raw);
                if let Some(&number) = self.designs.get(&operands) {
                    return Ok(number);
                }
                let number = self.designs.len() as u64;
                let cell = json(&*operands.0)?;
                self.designs.insert(operands, number);
                (DESIGNS, number, cell)
            }
        };
        if number >= RANGE {
            return Err(invalid("Division equipment operands exhausted their range"));
        }
        let cell = family::cell(writer, &cell)?;
        writer.catalogue(range + number, cell)?;
        Ok(number)
    }

    /// The number of the pair of catalogued `kind` and `handle`.
    fn pair(&mut self, writer: &mut Writer<'_>, kind: u64, handle: u64) -> io::Result<u64> {
        if let Some(&number) = self.pairs.get(&(kind, handle)) {
            return Ok(number);
        }
        let number = self.pairs.len() as u64;
        if number >= RANGE {
            return Err(invalid("Division equipment designs exhausted their range"));
        }
        let cell = family::cell(
            writer,
            &Scalar::List(vec![Scalar::Unsigned(kind), Scalar::Unsigned(handle)]),
        )?;
        writer.catalogue(PAIRS + number, cell)?;
        self.pairs.insert((kind, handle), number);
        Ok(number)
    }

    /// Keeps one projected item.
    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("division_equipment.emit", None);
        match hour.projected {
            Projected::Lead(input) => {
                let _cpu = ::replay_frame::cpu::scope("division_equipment.emit.lead", None);
                let (engine, _) = self.handed.as_mut().ok_or_else(|| {
                    io::Error::other("A division equipment segment leads with no engine before it")
                })?;
                let output = engine.step(&input)?.ok_or_else(|| {
                    io::Error::other("The division equipment engine handed on cannot answer")
                })?;
                self.output(writer, output, true)
            }
            Projected::Hour(output) => self.output(writer, *output, false),
            // A segment that answered none of its hours leaves the engine before to answer
            // on; one that did hands its own on, with its numbering.
            Projected::Handover(engine) => {
                let numbers = std::mem::take(&mut self.current);
                if let Some(engine) = engine {
                    self.handed = Some((engine, numbers));
                }
                Ok(())
            }
        }
    }

    fn output(&mut self, writer: &mut Writer<'_>, output: Output, handed: bool) -> io::Result<()> {
        let at = output.hour;
        let adjacent = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if let Some(last) = self.last.filter(|_| !adjacent) {
            // An hour not walked ends every value.
            self.carry.close(writer, last)?;
            self.open.clear();
            self.known = false;
        }
        self.last = Some(at);
        self.first.get_or_insert(at);
        if self.family.is_none() {
            let mut keys = [0; 3];
            for (measure, key) in keys.iter_mut().enumerate() {
                *key = self.key(
                    writer,
                    Place {
                        actor: FAMILY,
                        measure: measure as u64,
                        dimension: None,
                    },
                )?;
            }
            self.family = Some(keys);
        }
        let mut numbers = match &mut self.handed {
            Some((_, numbers)) if handed => std::mem::take(numbers),
            _ => std::mem::take(&mut self.current),
        };
        let result = self.apply(writer, output, &mut numbers);
        match &mut self.handed {
            Some((_, held)) if handed => *held = numbers,
            _ => self.current = numbers,
        }
        result
    }

    fn apply(
        &mut self,
        writer: &mut Writer<'_>,
        output: Output,
        numbers: &mut Numbers,
    ) -> io::Result<()> {
        let at = output.hour;
        for (id, operand) in output.operands {
            let number = self.catalogue(writer, operand)?;
            let at = id as usize;
            if at >= numbers.operands.len() {
                numbers.operands.resize(at + 1, None);
            }
            numbers.operands[at] = Some(number);
        }
        for (index, tag) in output.tags {
            let actor = self.actors.actor(writer, tag.as_bytes(), at)?;
            let index = index as usize;
            if index >= numbers.actors.len() {
                numbers.actors.resize(index + 1, None);
            }
            numbers.actors[index] = Some(actor);
        }
        let Some([pool, stock]) = output.known else {
            if self.known {
                // An unknown hour ends every value before it.
                self.carry.close(writer, at - 1)?;
                self.open.clear();
            }
            self.known = false;
            return Ok(());
        };
        let starts = !self.known;
        if starts && !output.whole {
            return Err(io::Error::other(
                "A division equipment start was not given every country",
            ));
        }
        let [known, pool_key, stock_key] = self.family.ok_or_else(|| invalid("No family keys"))?;
        self.carry.value(writer, known, at, Some(1), starts)?;
        self.carry
            .value(writer, pool_key, at, pool.map(i128::from), starts)?;
        self.carry
            .value(writer, stock_key, at, stock.map(i128::from), starts)?;
        if output.whole {
            self.generation += 1;
        }
        for change in output.changes.iter() {
            let Change {
                tag,
                side,
                field,
                value,
            } = change?;
            let actor = numbers.actor(tag)?;
            let kind = match field {
                Field::Type(kind, _) | Field::Variant(kind, _) | Field::Design(kind, _) => {
                    numbers.operand(kind)?
                }
                _ => 0,
            };
            let note = match field {
                Field::Note(note) => numbers.operand(note)?,
                _ => 0,
            };
            let pair = match field {
                Field::Variant(_, handle) | Field::Design(_, handle) => {
                    self.pair(writer, kind, handle)?
                }
                _ => 0,
            };
            let (measure, dimension) = place(side, field, kind, note, pair);
            let place = Place {
                actor,
                measure,
                dimension,
            };
            let value = match value {
                None => None,
                Some(Value::Number(value)) => Some(value),
                Some(Value::Operand(id)) => Some(i128::from(numbers.operand(id)?)),
            };
            let key = match value {
                Some(_) => self.key(writer, place)?,
                // A value never given has nothing to end.
                None => match self.actors.held(place) {
                    Some(key) => key,
                    None => continue,
                },
            };
            self.carry.value(writer, key, at, value, starts)?;
            match value {
                Some(_) => {
                    self.open.insert(key, self.generation);
                }
                None => {
                    self.open.remove(&key);
                }
            }
        }
        if output.whole {
            let generation = self.generation;
            let stale: Vec<KeyHandle> = self
                .open
                .iter()
                .filter(|&(_, &set)| set != generation)
                .map(|(&key, _)| key)
                .collect();
            for key in stale {
                self.carry.value(writer, key, at, None, false)?;
                self.open.remove(&key);
            }
        }
        self.known = true;
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, and the hours since the last cut as prepared.
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
        Ok(())
    }

    /// What the emit goes on with after a cut: its open values, its numbering and the engine
    /// handed on, which gives its next hour whole.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let saved = Saved {
            last: self.last,
            known: self.known,
            generation: self.generation,
            open: self.open.iter().map(|(&key, &set)| (key, set)).collect(),
            operands: self.current.operands.clone(),
            actors: self.current.actors.clone(),
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        let mut engine = Vec::new();
        if let Some((handed, _)) = &self.handed {
            handed.save(&mut engine)?;
            out.push(1);
            out.extend_from_slice(&(engine.len() as u64).to_le_bytes());
            out.extend_from_slice(&engine);
        } else {
            out.push(0);
        }
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(
        cat: Option<Arc<Catalogue>>,
        directory: &Directory,
        bytes: &[u8],
    ) -> io::Result<Self> {
        let damaged = || invalid("A saved division equipment emit is damaged");
        let mut emit = Self::new(cat);
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let mut family = [0; 3];
        for (measure, key) in family.iter_mut().enumerate() {
            *key = directory
                .key(FAMILY, measure as u64, None)
                .ok_or_else(damaged)?;
        }
        emit.family = Some(family);
        let text = |id: u64| -> io::Result<Option<&[u8]>> {
            Ok(match directory.scalar(id) {
                None => None,
                Some(scalar) => Some(scalar.text().ok_or_else(damaged)?),
            })
        };
        let mut number = 0;
        while let Some(scalar) = directory.scalar(KINDS + number) {
            let key = match scalar {
                Scalar::Null => None,
                scalar => Some(Arc::from(
                    std::str::from_utf8(scalar.text().ok_or_else(damaged)?)
                        .map_err(|_| damaged())?,
                )),
            };
            emit.kinds.insert(key, number);
            number += 1;
        }
        let mut number = 0;
        while let Some(json) = text(NOTES + number)? {
            emit.notes.insert(serde_json::from_slice(json)?, number);
            number += 1;
        }
        let mut number = 0;
        while let Some(json) = text(NAMINGS + number)? {
            emit.namings.insert(serde_json::from_slice(json)?, number);
            number += 1;
        }
        let mut number = 0;
        while let Some(json) = text(ARTWORKS + number)? {
            emit.artworks.insert(serde_json::from_slice(json)?, number);
            number += 1;
        }
        let mut number = 0;
        while let Some(json) = text(DESIGNS + number)? {
            emit.designs.insert(
                NamingOperands(Arc::new(serde_json::from_slice(json)?)),
                number,
            );
            number += 1;
        }
        let mut number = 0;
        while let Some(scalar) = directory.scalar(PAIRS + number) {
            let [kind, handle] = scalar.list() else {
                return Err(damaged());
            };
            let field = |scalar: &Scalar| {
                scalar
                    .integer()
                    .and_then(|value| u64::try_from(value).ok())
                    .ok_or_else(damaged)
            };
            emit.pairs.insert((field(kind)?, field(handle)?), number);
            number += 1;
        }
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| damaged())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        emit.last = saved.last;
        emit.known = saved.known;
        emit.generation = saved.generation;
        emit.open = saved.open.into_iter().collect();
        emit.current = Numbers {
            operands: saved.operands,
            actors: saved.actors,
        };
        match d.u8()? {
            0 => {}
            1 => {
                let length = usize::try_from(d.u64()?).map_err(|_| damaged())?;
                // The engine numbers its operands and countries anew.
                let engine = Engine::load(emit.cat.clone(), d.bytes(length)?)?;
                emit.handed = Some((Box::new(engine), Numbers::default()));
            }
            _ => return Err(damaged()),
        }
        let at = d.position();
        emit.carry = dashboard_series::carry::Carry::load(&bytes[at..])?;
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    known: bool,
    generation: u64,
    open: Vec<(KeyHandle, u64)>,
    operands: Vec<Option<u64>>,
    actors: Vec<Option<u64>>,
}
