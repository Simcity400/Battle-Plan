//! One family's publication: its actors (countries or other entities, by name, numbered in
//! first-seen order), their keys by measure and dimension, and the family's own scalars. The
//! actors and scalars are catalogue cells ([`Writer::catalogue`]); keys are the dictionary's.
//! An emit keeps its handles in [`Actors`]; a reply reads them once per root in [`Directory`].

use crate::{
    codec::Binary,
    hash::FastMap,
    invalid,
    pages::PageRead,
    query::Snapshot,
    types::{Blob, Cell, KeyHandle, KeySpec, RunMode},
    writer::Writer,
};
use std::{collections::HashMap, io};

/// Catalogue ids from here on name actors: `ACTORS + actor`; family scalars lie below.
pub const ACTORS: u64 = 1 << 32;

/// Rows a directory read asks per page.
const PAGE: usize = 1 << 20;

/// A key's place in its family: its actor, measure and the dimension it is split by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    pub actor: u64,
    pub measure: u64,
    pub dimension: Option<u64>,
}

/// An emit's actors and keys, by name and by place.
#[derive(Debug, Default)]
pub struct Actors {
    schema: u64,
    names: FastMap<Box<[u8]>, u64>,
    keys: FastMap<Place, KeyHandle>,
    count: u64,
}

impl Actors {
    /// A family whose keys are spelled under `schema`.
    pub fn new(schema: u64) -> Self {
        Self {
            schema,
            ..Self::default()
        }
    }

    /// What a publication holds already: an emit going on after its last cut.
    pub fn resume(schema: u64, directory: &Directory) -> io::Result<Self> {
        let mut actors = Self::new(schema);
        for (id, actor) in directory.actors.iter().enumerate() {
            engine_allocation::reserve_map(&mut actors.names, 1)?;
            actors.names.insert(actor.name.clone(), id as u64);
        }
        actors.count = directory.actors.len() as u64;
        for (&place, &key) in &directory.keys {
            engine_allocation::reserve_map(&mut actors.keys, 1)?;
            actors.keys.insert(place, key);
        }
        Ok(actors)
    }

    /// The actor named `name`, numbered and catalogued at `hour` when it is new.
    pub fn actor(&mut self, writer: &mut Writer, name: &[u8], hour: i32) -> io::Result<u64> {
        if let Some(&actor) = self.names.get(name) {
            return Ok(actor);
        }
        let actor = self.count;
        let text = writer.blob(&mut &name[..], name.len() as u64)?;
        let cells = writer.cells([
            if name.is_empty() {
                Cell::Null
            } else {
                Cell::Text(text)
            },
            Cell::Integer(i128::from(hour)),
        ])?;
        writer.catalogue(
            ACTORS
                .checked_add(actor)
                .ok_or_else(|| invalid("series actor ids exhausted"))?,
            Cell::List(cells),
        )?;
        engine_allocation::reserve_map(&mut self.names, 1)?;
        self.names.insert(name.into(), actor);
        self.count += 1;
        Ok(actor)
    }

    /// The actor named `name`, if it was ever seen.
    pub fn find(&self, name: &[u8]) -> Option<u64> {
        self.names.get(name).copied()
    }

    /// The key at `place`, if it was ever interned.
    pub fn held(&self, place: Place) -> Option<KeyHandle> {
        self.keys.get(&place).copied()
    }

    /// The key at `place`, interned with `mode` and `decimals` when it is new: `bound` binds a
    /// new key to that shared evidence domain.
    pub fn key(
        &mut self,
        writer: &mut Writer,
        place: Place,
        (mode, decimals): (RunMode, u8),
        bound: Option<u64>,
    ) -> io::Result<KeyHandle> {
        if let Some(&key) = self.keys.get(&place) {
            return Ok(key);
        }
        let dimensions = match place.dimension {
            Some(dimension) => {
                let bytes = dimension.to_le_bytes();
                writer.blob(&mut &bytes[..], 8)?
            }
            None => Blob::EMPTY,
        };
        let key = writer.intern(
            KeySpec {
                schema: self.schema,
                actor: place.actor,
                measure: place.measure,
                dimensions,
            },
            Blob::EMPTY,
            mode,
            decimals,
        )?;
        if let Some(domain) = bound {
            writer.bind_domain(key, domain)?;
        }
        engine_allocation::reserve_map(&mut self.keys, 1)?;
        self.keys.insert(place, key);
        Ok(key)
    }

    pub fn len(&self) -> u64 {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// A scalar a family catalogued, read whole.
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    Null,
    Integer(i128),
    Unsigned(u64),
    Number(f64),
    Boolean(bool),
    Text(Box<[u8]>),
    List(Vec<Scalar>),
}

impl Scalar {
    pub fn text(&self) -> Option<&[u8]> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn integer(&self) -> Option<i128> {
        match *self {
            Self::Integer(value) => Some(value),
            Self::Unsigned(value) => Some(i128::from(value)),
            _ => None,
        }
    }

    pub fn list(&self) -> &[Scalar] {
        match self {
            Self::List(items) => items,
            _ => &[],
        }
    }
}

/// An actor as its family catalogued it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Actor {
    pub name: Box<[u8]>,
    /// The hour it was first seen.
    pub first_hour: i32,
}

/// A publication's actors, keys and scalars, read once.
#[derive(Debug, Default)]
pub struct Directory {
    /// By actor id.
    pub actors: Vec<Actor>,
    names: HashMap<Box<[u8]>, u64>,
    keys: HashMap<Place, KeyHandle>,
    /// Each actor's split keys by measure, in dimension order.
    split: HashMap<(u64, u64), Vec<(u64, KeyHandle)>>,
    scalars: HashMap<u64, Scalar>,
}

impl Directory {
    /// The directory of the publication `snapshot` pins, for the family spelled `schema`.
    pub fn read<P: PageRead>(snapshot: &mut Snapshot<'_, P>, schema: u64) -> io::Result<Self> {
        let mut directory = Self::default();
        let mut cells = Vec::new();
        let mut cursor = None;
        loop {
            let page = snapshot.metadata_page(cursor, PAGE * (Cell::SIZE + 8), |id, cell| {
                engine_allocation::reserve(&mut cells, 1)?;
                cells.push((id, cell));
                Ok(())
            })?;
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        for (id, cell) in cells {
            let value = scalar(snapshot, cell, 0)?;
            if id < ACTORS {
                engine_allocation::reserve_map(&mut directory.scalars, 1)?;
                directory.scalars.insert(id, value);
                continue;
            }
            let actor = id - ACTORS;
            if actor != directory.actors.len() as u64 {
                return Err(invalid("series actors are not dense"));
            }
            let (name, first_hour) = match value.list() {
                [name, first] => (
                    name.text().unwrap_or_default(),
                    first
                        .integer()
                        .and_then(|hour| i32::try_from(hour).ok())
                        .ok_or_else(|| invalid("series actor first hour"))?,
                ),
                _ => return Err(invalid("series actor cell")),
            };
            engine_allocation::reserve(&mut directory.actors, 1)?;
            engine_allocation::reserve_map(&mut directory.names, 1)?;
            directory.names.insert(name.into(), actor);
            directory.actors.push(Actor {
                name: name.into(),
                first_hour,
            });
        }
        let mut definitions = Vec::new();
        let mut cursor = None;
        loop {
            let page = snapshot.catalogue_page(None, cursor, PAGE * 72, |definition| {
                engine_allocation::reserve(&mut definitions, 1)?;
                definitions.push(definition);
                Ok(())
            })?;
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        for definition in definitions {
            let spec = definition.spec;
            if spec.schema != schema {
                continue;
            }
            let dimension = match spec.dimensions.len {
                0 => None,
                8 => {
                    let mut bytes = [0; 8];
                    snapshot.cell_bytes(spec.dimensions, 0, &mut bytes)?;
                    Some(u64::from_le_bytes(bytes))
                }
                _ => return Err(invalid("series key dimension")),
            };
            if let Some(dimension) = dimension {
                let split = directory
                    .split
                    .entry((spec.actor, spec.measure))
                    .or_default();
                engine_allocation::reserve(split, 1)?;
                split.push((dimension, definition.id));
            }
            engine_allocation::reserve_map(&mut directory.keys, 1)?;
            directory.keys.insert(
                Place {
                    actor: spec.actor,
                    measure: spec.measure,
                    dimension,
                },
                definition.id,
            );
        }
        for split in directory.split.values_mut() {
            split.sort_unstable();
        }
        Ok(directory)
    }

    /// The actor named `name`.
    pub fn actor(&self, name: &[u8]) -> Option<u64> {
        self.names.get(name).copied()
    }

    /// The key of `actor`'s `measure`, split by `dimension`.
    pub fn key(&self, actor: u64, measure: u64, dimension: Option<u64>) -> Option<KeyHandle> {
        self.keys
            .get(&Place {
                actor,
                measure,
                dimension,
            })
            .copied()
    }

    /// Every dimension of `actor`'s `measure` with its key, in dimension order.
    pub fn dimensions(&self, actor: u64, measure: u64) -> &[(u64, KeyHandle)] {
        self.split.get(&(actor, measure)).map_or(&[], Vec::as_slice)
    }

    pub fn scalar(&self, id: u64) -> Option<&Scalar> {
        self.scalars.get(&id)
    }
}

/// `cell` read whole, lists `depth` deep at most.
fn scalar<P: PageRead>(
    snapshot: &mut Snapshot<'_, P>,
    cell: Cell,
    depth: u8,
) -> io::Result<Scalar> {
    Ok(match cell {
        Cell::Null => Scalar::Null,
        Cell::Integer(value) => Scalar::Integer(value),
        Cell::Unsigned(value) => Scalar::Unsigned(value),
        Cell::Number(value) => Scalar::Number(value),
        Cell::Boolean(value) => Scalar::Boolean(value),
        Cell::Text(blob) => {
            let length = usize::try_from(blob.len).map_err(|_| invalid("series text length"))?;
            let mut text = engine_allocation::filled(length, 0_u8)?;
            snapshot.cell_bytes(blob, 0, &mut text)?;
            Scalar::Text(text.into_boxed_slice())
        }
        Cell::List(blob) => {
            if depth > 4 {
                return Err(invalid("series cells nest too deep"));
            }
            let count = blob.len / Cell::SIZE as u64;
            let mut items = Vec::new();
            for at in 0..count {
                let item = snapshot.field(blob, at)?;
                engine_allocation::reserve(&mut items, 1)?;
                items.push(scalar(snapshot, item, depth + 1)?);
            }
            Scalar::List(items)
        }
    })
}

/// Writes `scalar` as a catalogue cell's value.
pub fn cell(writer: &mut Writer, scalar: &Scalar) -> io::Result<Cell> {
    Ok(match scalar {
        Scalar::Null => Cell::Null,
        Scalar::Integer(value) => Cell::Integer(*value),
        Scalar::Unsigned(value) => Cell::Unsigned(*value),
        Scalar::Number(value) => Cell::Number(*value),
        Scalar::Boolean(value) => Cell::Boolean(*value),
        Scalar::Text(text) if text.is_empty() => Cell::Text(Blob::EMPTY),
        Scalar::Text(text) => Cell::Text(writer.blob(&mut &text[..], text.len() as u64)?),
        Scalar::List(items) => {
            let mut cells = Vec::with_capacity(items.len());
            for item in items {
                cells.push(cell(writer, item)?);
            }
            Cell::List(writer.cells(cells)?)
        }
    })
}
