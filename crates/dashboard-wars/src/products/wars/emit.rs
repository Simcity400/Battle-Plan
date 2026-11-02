//! Wars' emit: the change inputs as events of the family's INPUT key, the hours each input covers
//! as runs of its ordinal on COVERAGE, each war relation lifetime's four casualty counters as
//! runs of its actor, and the participation tables' parts as events of WHOLES (both tables
//! whole) and DIFFS.
//!
//! An input is published where a table changed in a compared field since the input before, at
//! the first hour after one not walked, and where a captured capitulation's date is reached. A
//! segment's first hour is compared with the tables the emit holds, so its whole read publishes
//! only what changed. An hour not walked ends every run.
//!
//! The emit holds each input table as the packet last publishing it, unpacked only when a
//! segment's first hour compares it.

use super::facts::{self, Relations};
use super::inputs::{self, INPUTS};
use super::*;
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    codec::Decode,
    family::{Actors, Directory, Place},
};
use replay_frame::frame::SharedView;
use std::{io::Read as _, sync::Arc};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// The family's keys: the change inputs, the ordinal of the input covering each walked hour,
/// and the participation parts, whole and as diffs.
pub const INPUT: u64 = 0;
pub const COVERAGE: u64 = 1;
pub const WHOLES: u64 = 2;
pub const DIFFS: u64 = 3;
/// A relation lifetime's counters, by measure: its first endpoint's casualties and unknown
/// casualties, then its second's.
pub const COUNTERS: u64 = 4;
/// Every key's evidence: each walked hour is known.
const DOMAIN: u64 = 0;

/// Event kinds: an input ([`inputs::Input`]), or packed participation parts, each the event's
/// one text cell.
pub mod kind {
    pub const INPUT: u64 = 0;
    pub const PARTS: u64 = 1;
}

fn saved_invalid() -> io::Error {
    invalid("Saved wars emit is not valid")
}

/// An input table as the emit holds it: the packet that last published it, and its view once
/// unpacked.
struct Held {
    packet: Arc<Vec<u8>>,
    view: Option<SharedView>,
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// INPUT, COVERAGE, WHOLES and DIFFS.
    family: Option<[KeyHandle; 4]>,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
    /// Each input table as last published, by [`INPUTS`] place.
    held: Vec<Option<Held>>,
    /// The ordinal of the last input publishing each input table.
    refs: Vec<u32>,
    published: u32,
    /// The next captured capitulation's date.
    next: Option<i32>,
    /// The relations last joined, in row order, with their counters' keys.
    relations: Vec<[KeyHandle; 4]>,
    /// An input's bytes before its packet, and what a segment's first hour packs, kept from
    /// input to input.
    head: Vec<u8>,
    scratch: Vec<u8>,
}

impl Default for Emit {
    fn default() -> Self {
        Self::new()
    }
}

/// The text cell of an event's bytes, `head` then `tail`.
fn text_cell(
    writer: &mut Writer<'_>,
    head: &[u8],
    tail: &[u8],
) -> io::Result<dashboard_series::Blob> {
    let len = (head.len() + tail.len()) as u64;
    let blob = writer.blob(&mut head.chain(tail), len)?;
    writer.cells([Cell::Text(blob)])
}

impl Emit {
    pub fn new() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            family: None,
            last: None,
            first: None,
            held: (0..INPUTS.len()).map(|_| None).collect(),
            refs: vec![u32::MAX; INPUTS.len()],
            published: 0,
            next: None,
            relations: Vec::new(),
            head: Vec::new(),
            scratch: Vec::new(),
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))
    }

    fn family(&mut self, writer: &mut Writer<'_>) -> io::Result<[KeyHandle; 4]> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let mut keys = [0; 4];
        for (key, measure) in keys.iter_mut().zip([INPUT, COVERAGE, WHOLES, DIFFS]) {
            *key = self.key(
                writer,
                Place {
                    actor: FAMILY,
                    measure,
                    dimension: None,
                },
            )?;
        }
        self.family = Some(keys);
        Ok(keys)
    }

    /// The counters' keys of the lifetime `relation`, named at `hour` when it is new.
    fn relation(
        &mut self,
        writer: &mut Writer<'_>,
        relation: &WarRelationKey,
        hour: i32,
    ) -> io::Result<[KeyHandle; 4]> {
        let actor = self
            .actors
            .actor(writer, &inputs::relation_name(relation), hour)?;
        let mut keys = [0; 4];
        for (field, key) in keys.iter_mut().enumerate() {
            *key = self.key(
                writer,
                Place {
                    actor,
                    measure: COUNTERS + field as u64,
                    dimension: None,
                },
            )?;
        }
        Ok(keys)
    }

    fn event(
        writer: &mut Writer<'_>,
        key: KeyHandle,
        at: i32,
        kind: u64,
        (head, tail): (&[u8], &[u8]),
    ) -> io::Result<()> {
        let fields = text_cell(writer, head, tail)?;
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

    /// Unpacks every held table not yet unpacked, each packet once.
    fn unpack_held(&mut self) -> io::Result<()> {
        for place in 0..INPUTS.len() {
            let Some(packet) = self.held[place]
                .as_ref()
                .filter(|held| held.view.is_none())
                .map(|held| Arc::clone(&held.packet))
            else {
                continue;
            };
            let frame = inputs::unpack(&packet, 0, 0)?;
            for (held, &table) in self.held.iter_mut().zip(INPUTS) {
                if let Some(held) = held
                    && held.view.is_none()
                    && Arc::ptr_eq(&held.packet, &packet)
                {
                    let view = &frame.tables[table.index()];
                    if view.is_not_selected() {
                        return Err(saved_invalid());
                    }
                    held.view = Some(view.clone());
                }
            }
        }
        Ok(())
    }

    pub fn hour(&mut self, writer: &mut Writer<'_>, hour: facts::Hour) -> io::Result<()> {
        let _cpu = ::replay_frame::cpu::scope("wars.emit", None);
        let at = hour.hour;
        let gap = match self.last {
            Some(last) if last.checked_add(1) == Some(at) => false,
            Some(last) if at <= last => {
                return Err(invalid("A wars hour does not follow the hour before"));
            }
            // An hour not walked ends every run.
            Some(last) => {
                self.carry.close(writer, last)?;
                self.relations.clear();
                true
            }
            None => true,
        };
        self.last = Some(at);
        self.first.get_or_insert(at);
        let [input_key, coverage, wholes, diffs] = self.family(writer)?;
        let dated = self.next.is_some_and(|next| next <= at);
        let mut changed = 0_u32;
        let mut packet: Option<Arc<Vec<u8>>> = None;
        let mut next = None;
        let mut relations = None;
        let mut parts = None;
        if let Some(facts) = hour.facts {
            let facts::Facts {
                whole,
                changed: moved,
                packet: packed,
                next: read,
                relations: joined,
                parts: packed_parts,
            } = *facts;
            next = read;
            relations = joined;
            parts = packed_parts;
            match whole {
                Some(views) => {
                    self.unpack_held()?;
                    let mut publish = Vec::new();
                    for (table, view) in views {
                        let place = inputs::input(table).ok_or_else(saved_invalid)?;
                        let same = self.held[place]
                            .as_ref()
                            .and_then(|held| held.view.as_ref())
                            .is_some_and(|held| inputs::same_inputs(table, held, &view));
                        if !same {
                            changed |= 1 << place;
                            publish.push((place, view));
                        }
                    }
                    if !publish.is_empty() {
                        let views = publish.iter().map(|(place, view)| (INPUTS[*place], view));
                        let bytes = Arc::new(inputs::pack(views, &mut self.scratch)?);
                        for (place, view) in publish {
                            self.held[place] = Some(Held {
                                packet: Arc::clone(&bytes),
                                view: Some(view),
                            });
                        }
                        packet = Some(bytes);
                    }
                }
                None if moved != 0 => {
                    let bytes = Arc::new(packed.ok_or_else(saved_invalid)?);
                    for place in inputs::places(moved) {
                        self.held[place] = Some(Held {
                            packet: Arc::clone(&bytes),
                            view: None,
                        });
                    }
                    changed = moved;
                    packet = Some(bytes);
                }
                None => {}
            }
        }
        if gap || changed != 0 || dated {
            let ordinal = self.published;
            for place in inputs::places(changed) {
                self.refs[place] = ordinal;
            }
            self.head.clear();
            inputs::put_input(&mut self.head, ordinal, changed, &self.refs)?;
            let tail = packet.as_deref().map_or(&[][..], Vec::as_slice);
            Self::event(writer, input_key, at, kind::INPUT, (&self.head, tail))?;
            self.carry
                .value(writer, coverage, at, Some(i128::from(ordinal)), gap)?;
            self.published = ordinal
                .checked_add(1)
                .ok_or_else(|| invalid("Too many wars change inputs"))?;
        }
        // The operator dates the next capitulation wherever this emit would: at a segment's
        // first hour, where the capitulations are read again, and where `dated`.
        if let Some(next) = next {
            self.next = next;
        } else if dated {
            self.unpack_held()?;
            let place = inputs::input(TableId::Capitulation).ok_or_else(saved_invalid)?;
            self.next = self.held[place]
                .as_ref()
                .and_then(|held| held.view.as_ref())
                .and_then(|view| inputs::next_capitulation(view, at));
        }
        match relations {
            Some(Relations::Whole(list)) => {
                let mut keys = Vec::with_capacity(list.len());
                for (relation, _) in &list {
                    keys.push(self.relation(writer, relation, at)?);
                }
                let present: std::collections::HashSet<KeyHandle> =
                    keys.iter().map(|keys| keys[0]).collect();
                for held in std::mem::take(&mut self.relations) {
                    if !present.contains(&held[0]) {
                        for key in held {
                            self.carry.value(writer, key, at, None, false)?;
                        }
                    }
                }
                for (held, (_, values)) in keys.iter().zip(&list) {
                    for (&key, &value) in held.iter().zip(values) {
                        self.carry
                            .value(writer, key, at, Some(i128::from(value)), false)?;
                    }
                }
                self.relations = keys;
            }
            Some(Relations::Values(values)) => {
                for (place, values) in values {
                    let held = *self
                        .relations
                        .get(place as usize)
                        .ok_or_else(|| invalid("A wars counter names no joined relation"))?;
                    for (key, value) in held.into_iter().zip(values) {
                        self.carry
                            .value(writer, key, at, Some(i128::from(value)), false)?;
                    }
                }
            }
            None => {}
        }
        if let Some((packed, whole)) = parts {
            let key = if whole { wholes } else { diffs };
            Self::event(writer, key, at, kind::PARTS, (&packed, &[] as &[u8]))?;
        }
        Ok(())
    }

    /// Appends what is open through `last`, the last hour of the segment the publisher cuts
    /// next, and the hours since the last cut as known.
    pub fn cut(&mut self, writer: &mut Writer<'_>, last: i32) -> io::Result<()> {
        if self.last.is_none_or(|walked| last > walked) {
            return Err(invalid("A wars cut is past the last walked hour"));
        }
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

    /// What the emit goes on with after a cut: its state, the packets holding its tables, each
    /// once, then its carry.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        let mut packets: Vec<&Arc<Vec<u8>>> = Vec::new();
        let mut held = Vec::new();
        for (held_table, &table) in self.held.iter().zip(INPUTS) {
            let Some(held_table) = held_table else {
                continue;
            };
            let at = match packets
                .iter()
                .position(|packet| Arc::ptr_eq(packet, &held_table.packet))
            {
                Some(at) => at,
                None => {
                    packets.push(&held_table.packet);
                    packets.len() - 1
                }
            };
            held.push((table as u16, at as u32));
        }
        let saved = Saved {
            last: self.last,
            refs: self.refs.clone(),
            published: self.published,
            next: self.next,
            relations: self.relations.clone(),
            held,
        };
        let json = serde_json::to_vec(&saved)?;
        out.extend_from_slice(&(json.len() as u64).to_le_bytes());
        out.extend_from_slice(&json);
        out.extend_from_slice(&(packets.len() as u64).to_le_bytes());
        for packet in packets {
            out.extend_from_slice(&(packet.len() as u64).to_le_bytes());
            out.extend_from_slice(packet);
        }
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.actors = Actors::resume(SCHEMA, directory)?;
        let mut family = [0; 4];
        for (key, measure) in family.iter_mut().zip([INPUT, COVERAGE, WHOLES, DIFFS]) {
            *key = directory
                .key(FAMILY, measure, None)
                .ok_or_else(saved_invalid)?;
        }
        emit.family = Some(family);
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| saved_invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let count = usize::try_from(d.u64()?).map_err(|_| saved_invalid())?;
        let mut packets = Vec::new();
        for _ in 0..count {
            let length = usize::try_from(d.u64()?).map_err(|_| saved_invalid())?;
            packets.push(Arc::new(d.bytes(length)?.to_vec()));
        }
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        if saved.refs.len() != INPUTS.len() {
            return Err(saved_invalid());
        }
        emit.last = saved.last;
        emit.refs = saved.refs;
        emit.published = saved.published;
        emit.next = saved.next;
        emit.relations = saved.relations;
        for (table, packet) in saved.held {
            let place = INPUTS
                .iter()
                .position(|&held| held as u16 == table)
                .ok_or_else(saved_invalid)?;
            let packet = packets
                .get(packet as usize)
                .ok_or_else(saved_invalid)?;
            emit.held[place] = Some(Held {
                packet: Arc::clone(packet),
                view: None,
            });
        }
        Ok(emit)
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    last: Option<i32>,
    refs: Vec<u32>,
    published: u32,
    next: Option<i32>,
    relations: Vec<[KeyHandle; 4]>,
    /// Each held table and the place of the packet holding it.
    held: Vec<(u16, u32)>,
}
