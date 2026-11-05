//! Wars' emit: the hours walked as runs of WALKED; the hours each set of input tables covers as
//! runs of its ordinal on COVERAGE; the prepared layers ([`super::layers`]) as events where they
//! change; each war relation lifetime's four casualty counters as runs of its actor; and the
//! participation tables' parts as events of WHOLES (both tables whole) and DIFFS.
//!
//! A coverage run starts where an input table changed in a compared field since the run before,
//! at the first hour after one not walked, and where a captured capitulation's date is reached
//! (DATED): the hours a walk reads its tables again, at which it words a refused hour again too.
//! A segment's first hour is compared with the tables the emit holds, so its whole read starts a
//! run only where something changed. An hour not walked ends every run. A run's value is its
//! ordinal, doubled, and one more where a table a walk names its groups from changed ([`renames`]):
//! the hours it words a refused name again.
//!
//! Each viewpoint at war is an actor whose GROUPS events list its groups as the ordinals of
//! their members (MEMBERS) and labels (LABELS), each defined once while some list holds it.
//!
//! The emit holds each input table as the packet last changing it, unpacked only when a
//! segment's first hour compares it.

use super::facts::{self, Relations};
use super::inputs::{self, INPUTS};
use super::layers::{
    self, Capitulations, Group, Groups, Identity, Label, Layered, Members, Out, Relation, Roster,
    State,
};
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
pub const SCHEMA: u64 = 2;
/// The actor of the family's own keys.
pub const FAMILY: u64 = u64::MAX;
/// The family's keys: the ordinal of the tables covering each walked hour, the participation
/// parts, whole and as diffs, and the hours walked.
pub const COVERAGE: u64 = 0;
pub const WHOLES: u64 = 1;
pub const DIFFS: u64 = 2;
pub const WALKED: u64 = 3;
/// A relation lifetime's counters, by measure: its first endpoint's casualties and unknown
/// casualties, then its second's.
pub const COUNTERS: u64 = 4;
/// The layers every viewpoint reads alike, each an event where it changed: whether the overview
/// was recorded, the countries, the war tables' state, the active relations, the capitulations,
/// the hours reaching a captured capitulation's date and the historical names first read.
pub const AVAILABLE: u64 = 8;
pub const ROSTER: u64 = 9;
pub const STATE: u64 = 10;
pub const RELATIONS: u64 = 11;
pub const CAPITULATIONS: u64 = 12;
pub const DATED: u64 = 13;
pub const NAMES: u64 = 14;
/// The groups' members and labels, the n-th event of each defining its ordinal n.
pub const MEMBERS: u64 = 15;
pub const LABELS: u64 = 16;
/// A viewpoint actor's groups.
pub const GROUPS: u64 = 17;
/// Every key's evidence: each walked hour is known.
const DOMAIN: u64 = 0;

/// The input tables whose change makes a walk name its groups again: all but the campaign and the
/// capitulations, which it reads apart from them.
fn renames() -> u32 {
    [TableId::Campaign, TableId::Capitulation]
        .into_iter()
        .filter_map(inputs::input)
        .fold((1 << INPUTS.len()) - 1, |set, place| set & !(1 << place))
}

/// The family's own keys, in [`Self::MEASURES`] order.
#[derive(Clone, Copy)]
pub(crate) struct Keys {
    pub coverage: KeyHandle,
    pub wholes: KeyHandle,
    pub diffs: KeyHandle,
    pub walked: KeyHandle,
    pub available: KeyHandle,
    pub roster: KeyHandle,
    pub state: KeyHandle,
    pub relations: KeyHandle,
    pub capitulations: KeyHandle,
    pub dated: KeyHandle,
    pub names: KeyHandle,
    pub members: KeyHandle,
    pub labels: KeyHandle,
}

impl Keys {
    const MEASURES: [u64; 13] = [
        COVERAGE,
        WHOLES,
        DIFFS,
        WALKED,
        AVAILABLE,
        ROSTER,
        STATE,
        RELATIONS,
        CAPITULATIONS,
        DATED,
        NAMES,
        MEMBERS,
        LABELS,
    ];
    fn from(keys: [KeyHandle; 13]) -> Self {
        let [
            coverage,
            wholes,
            diffs,
            walked,
            available,
            roster,
            state,
            relations,
            capitulations,
            dated,
            names,
            members,
            labels,
        ] = keys;
        Self {
            coverage,
            wholes,
            diffs,
            walked,
            available,
            roster,
            state,
            relations,
            capitulations,
            dated,
            names,
            members,
            labels,
        }
    }
    /// The keys `directory` spells.
    pub(crate) fn of(directory: &Directory) -> io::Result<Self> {
        let mut keys = [0; 13];
        for (key, measure) in keys.iter_mut().zip(Self::MEASURES) {
            *key = directory
                .key(FAMILY, measure, None)
                .ok_or_else(|| invalid("The wars publication is missing a key"))?;
        }
        Ok(Self::from(keys))
    }
}

/// A viewpoint identity's actor name.
pub(crate) fn viewpoint_name((slot, id): Identity) -> [u8; 9] {
    let mut name = [0; 9];
    name[0] = b'v';
    name[1..5].copy_from_slice(&slot.to_le_bytes());
    name[5..9].copy_from_slice(&id.to_le_bytes());
    name
}

/// Event kinds: packed participation parts, or a layer, each the event's one text cell.
pub mod kind {
    pub const PARTS: u64 = 1;
    pub const LAYER: u64 = 2;
}

fn saved_invalid() -> io::Error {
    invalid("Saved wars emit is not valid")
}

/// An input table as the emit holds it: the packet that last changed it, and its view once
/// unpacked.
struct Held {
    packet: Arc<Vec<u8>>,
    view: Option<SharedView>,
}

/// The records the published lists hold: each one's ordinal, defined once while some list holds
/// it, and how many lists hold it.
struct Records<T> {
    ordinals: HashMap<T, u32>,
    held: HashMap<u32, (T, u32)>,
    defined: u32,
}

impl<T: Clone + Eq + std::hash::Hash> Records<T> {
    fn new() -> Self {
        Self {
            ordinals: HashMap::new(),
            held: HashMap::new(),
            defined: 0,
        }
    }

    fn get(&self, record: &T) -> Option<u32> {
        self.ordinals.get(record).copied()
    }

    /// The ordinal `record` is held under, defined anew with `define` when none holds it.
    fn ordinal(
        &mut self,
        record: &T,
        define: &mut dyn FnMut(&T) -> io::Result<()>,
    ) -> io::Result<u32> {
        if let Some(ordinal) = self.get(record) {
            return Ok(ordinal);
        }
        define(record)?;
        let ordinal = self.defined;
        self.defined = ordinal
            .checked_add(1)
            .ok_or_else(|| invalid("Too many wars group records"))?;
        self.ordinals.insert(record.clone(), ordinal);
        self.held.insert(ordinal, (record.clone(), 0));
        Ok(ordinal)
    }

    /// One more list holding `ordinal`, or one fewer: a record none holds is let go.
    fn hold(&mut self, ordinal: u32, more: bool) {
        let Some(entry) = self.held.get_mut(&ordinal) else {
            return;
        };
        if more {
            entry.1 += 1;
        } else {
            entry.1 = entry.1.saturating_sub(1);
        }
        if entry.1 == 0
            && let Some((record, _)) = self.held.remove(&ordinal)
        {
            self.ordinals.remove(&record);
        }
    }

    fn save(&self, out: &mut Out, encode: impl Fn(&T) -> Vec<u8>) {
        out.word(self.defined);
        let mut held: Vec<_> = self.held.iter().collect();
        held.sort_unstable_by_key(|(ordinal, _)| **ordinal);
        out.var(held.len() as u64);
        for (&ordinal, (record, holders)) in held {
            out.word(ordinal);
            out.word(*holders);
            out.bytes(&encode(record));
        }
    }

    fn load(d: &mut Decode<'_>, decode: impl Fn(&[u8]) -> io::Result<T>) -> io::Result<Self> {
        let mut records = Self::new();
        records.defined = layers::word(d)?;
        for _ in 0..d.var()? {
            let (ordinal, holders) = (layers::word(d)?, layers::word(d)?);
            let record = decode(&layers::bytes(d)?)?;
            records.ordinals.insert(record.clone(), ordinal);
            records.held.insert(ordinal, (record, holders));
        }
        Ok(records)
    }
}

/// The family's emit on one branch.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    family: Option<Keys>,
    last: Option<i32>,
    /// The first hour since the last cut.
    first: Option<i32>,
    /// Each input table as last changed, by [`INPUTS`] place.
    held: Vec<Option<Held>>,
    /// The coverage runs started.
    published: u32,
    /// [`renames`].
    renames: u32,
    /// The next captured capitulation's date.
    next: Option<i32>,
    /// The relations last joined, in row order, with their counters' keys.
    relations: Vec<[KeyHandle; 4]>,
    /// What a segment's first hour packs, kept from hour to hour.
    scratch: Vec<u8>,
    /// Each shared layer as last published.
    available: Option<bool>,
    roster: Option<Roster>,
    state: Option<State>,
    graph: Option<Vec<Relation>>,
    capitulations: Option<Capitulations>,
    /// Every historical name published, by raw country id.
    named: HashSet<(u32, String)>,
    /// Each viewpoint at war's published groups, as members' and labels' ordinals.
    lists: HashMap<Identity, Vec<(u32, u32)>>,
    members: Records<Members>,
    labels: Records<Label>,
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
            published: 0,
            renames: renames(),
            next: None,
            relations: Vec::new(),
            scratch: Vec::new(),
            available: None,
            roster: None,
            state: None,
            graph: None,
            capitulations: None,
            named: HashSet::new(),
            lists: HashMap::new(),
            members: Records::new(),
            labels: Records::new(),
        }
    }

    fn key(&mut self, writer: &mut Writer<'_>, place: Place) -> io::Result<KeyHandle> {
        self.actors
            .key(writer, place, (RunMode::Exact, 0), Some(DOMAIN))
    }

    fn family(&mut self, writer: &mut Writer<'_>) -> io::Result<Keys> {
        if let Some(keys) = self.family {
            return Ok(keys);
        }
        let mut keys = [0; 13];
        for (key, measure) in keys.iter_mut().zip(Keys::MEASURES) {
            *key = self.key(
                writer,
                Place {
                    actor: FAMILY,
                    measure,
                    dimension: None,
                },
            )?;
        }
        let keys = Keys::from(keys);
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

    fn layer(writer: &mut Writer<'_>, key: KeyHandle, at: i32, bytes: &[u8]) -> io::Result<()> {
        Self::event(writer, key, at, kind::LAYER, (bytes, &[]))
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
        let keys = self.family(writer)?;
        let dated = self.next.is_some_and(|next| next <= at);
        let mut changed = 0_u32;
        let mut next = None;
        let mut relations = None;
        let mut parts = None;
        let mut layered = None;
        if let Some(facts) = hour.facts {
            let facts::Facts {
                whole,
                changed: moved,
                packet: packed,
                next: read,
                relations: joined,
                parts: packed_parts,
                layers,
            } = *facts;
            next = read;
            relations = joined;
            parts = packed_parts;
            layered = layers;
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
                }
                None => {}
            }
        }
        if gap {
            self.carry.value(writer, keys.walked, at, Some(1), true)?;
        }
        if gap || changed != 0 || dated {
            let ordinal = self.published;
            let renamed = changed & self.renames != 0;
            let value = (i128::from(ordinal) << 1) | i128::from(renamed);
            self.carry
                .value(writer, keys.coverage, at, Some(value), gap)?;
            self.published = ordinal
                .checked_add(1)
                .ok_or_else(|| invalid("Too many wars coverage runs"))?;
        }
        if dated {
            Self::layer(writer, keys.dated, at, &[0])?;
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
        if let Some(layered) = layered {
            self.layers(writer, keys, at, *layered)?;
        }
        match relations {
            Some(Relations::Whole(list)) => {
                let mut held_keys = Vec::with_capacity(list.len());
                for (relation, _) in &list {
                    held_keys.push(self.relation(writer, relation, at)?);
                }
                let present: std::collections::HashSet<KeyHandle> =
                    held_keys.iter().map(|keys| keys[0]).collect();
                for held in std::mem::take(&mut self.relations) {
                    if !present.contains(&held[0]) {
                        for key in held {
                            self.carry.value(writer, key, at, None, false)?;
                        }
                    }
                }
                for (held, (_, values)) in held_keys.iter().zip(&list) {
                    for (&key, &value) in held.iter().zip(values) {
                        self.carry
                            .value(writer, key, at, Some(i128::from(value)), false)?;
                    }
                }
                self.relations = held_keys;
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
            let key = if whole { keys.wholes } else { keys.diffs };
            Self::event(writer, key, at, kind::PARTS, (&packed, &[] as &[u8]))?;
        }
        Ok(())
    }

    /// Publishes what `layered` changed of the layers at `at`.
    fn layers(
        &mut self,
        writer: &mut Writer<'_>,
        keys: Keys,
        at: i32,
        layered: Layered,
    ) -> io::Result<()> {
        let Layered {
            roster,
            state,
            relations,
            capitulations,
            available,
            names,
            groups,
        } = layered;
        if let Some(available) = available
            && self.available != Some(available)
        {
            Self::layer(writer, keys.available, at, &[u8::from(available)])?;
            self.available = Some(available);
        }
        if let Some(roster) = roster
            && self.roster.as_ref() != Some(&roster)
        {
            Self::layer(writer, keys.roster, at, &roster.encode()?)?;
            self.roster = Some(roster);
        }
        if let Some(state) = state
            && self.state.as_ref() != Some(&state)
        {
            Self::layer(writer, keys.state, at, &state.encode())?;
            self.state = Some(state);
        }
        if let Some(relations) = relations
            && self.graph.as_ref() != Some(&relations)
        {
            Self::layer(
                writer,
                keys.relations,
                at,
                &layers::encode_relations(&relations)?,
            )?;
            self.graph = Some(relations);
        }
        if let Some(capitulations) = capitulations
            && self.capitulations.as_ref() != Some(&capitulations)
        {
            Self::layer(
                writer,
                keys.capitulations,
                at,
                &layers::encode_capitulations(&capitulations),
            )?;
            self.capitulations = Some(capitulations);
        }
        let names: Vec<_> = names
            .into_iter()
            .filter(|name| !self.named.contains(name))
            .collect();
        if !names.is_empty() {
            Self::layer(writer, keys.names, at, &layers::encode_names(&names))?;
            self.named.extend(names);
        }
        match groups {
            Some(Groups::Whole(lists)) => {
                let present: HashSet<Identity> =
                    lists.iter().map(|(identity, _)| *identity).collect();
                let mut gone: Vec<Identity> = self
                    .lists
                    .keys()
                    .filter(|identity| !present.contains(*identity))
                    .copied()
                    .collect();
                gone.sort_unstable();
                for identity in gone {
                    self.list(writer, keys, at, identity, &[])?;
                }
                for (identity, list) in lists {
                    self.list(writer, keys, at, identity, &list)?;
                }
            }
            Some(Groups::Changed(lists)) => {
                for (identity, list) in lists {
                    self.list(writer, keys, at, identity, &list)?;
                }
            }
            None => {}
        }
        Ok(())
    }

    /// Publishes `identity`'s groups at `at` where they differ from those published.
    fn list(
        &mut self,
        writer: &mut Writer<'_>,
        keys: Keys,
        at: i32,
        identity: Identity,
        groups: &[Group],
    ) -> io::Result<()> {
        let held = self.lists.get(&identity).map_or(&[][..], Vec::as_slice);
        let same = held.len() == groups.len()
            && held.iter().zip(groups).all(|(&(members, label), group)| {
                self.members.get(&group.members) == Some(members)
                    && self.labels.get(&group.label) == Some(label)
            });
        if same {
            return Ok(());
        }
        let mut list = Vec::with_capacity(groups.len());
        for group in groups {
            let members = self.members.ordinal(&group.members, &mut |members| {
                Self::layer(writer, keys.members, at, &members.encode())
            })?;
            let label = self.labels.ordinal(&group.label, &mut |label| {
                Self::layer(writer, keys.labels, at, &label.encode())
            })?;
            list.push((members, label));
        }
        // Each list holds its records before the one it replaces lets go of them.
        for &(members, label) in &list {
            self.members.hold(members, true);
            self.labels.hold(label, true);
        }
        for (members, label) in self.lists.remove(&identity).unwrap_or_default() {
            self.members.hold(members, false);
            self.labels.hold(label, false);
        }
        let actor = self.actors.actor(writer, &viewpoint_name(identity), at)?;
        let key = self.key(
            writer,
            Place {
                actor,
                measure: GROUPS,
                dimension: None,
            },
        )?;
        Self::layer(writer, key, at, &layers::encode_list(&list))?;
        if !list.is_empty() {
            self.lists.insert(identity, list);
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

    /// The layers as last published, and the records the published lists hold.
    fn save_layers(&self) -> io::Result<Vec<u8>> {
        let mut out = Out::default();
        out.flag(self.available.is_some());
        if let Some(available) = self.available {
            out.flag(available);
        }
        let parts: [Option<Vec<u8>>; 4] = [
            self.roster.as_ref().map(Roster::encode).transpose()?,
            self.state.as_ref().map(State::encode),
            self.graph
                .as_deref()
                .map(layers::encode_relations)
                .transpose()?,
            self.capitulations
                .as_ref()
                .map(layers::encode_capitulations),
        ];
        for part in &parts {
            out.flag(part.is_some());
            if let Some(part) = part {
                out.bytes(part);
            }
        }
        let mut named: Vec<_> = self.named.iter().cloned().collect();
        named.sort_unstable();
        out.bytes(&layers::encode_names(&named));
        self.members.save(&mut out, Members::encode);
        self.labels.save(&mut out, Label::encode);
        let mut lists: Vec<_> = self.lists.iter().collect();
        lists.sort_unstable_by_key(|(identity, _)| **identity);
        out.var(lists.len() as u64);
        for (&(slot, id), list) in lists {
            out.word(slot);
            out.word(id);
            out.bytes(&layers::encode_list(list));
        }
        Ok(out.into_bytes())
    }

    /// The layers [`Self::save_layers`] saved.
    fn load_layers(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut d = Decode::new(bytes);
        let d = &mut d;
        self.available = if d.flag()? { Some(d.flag()?) } else { None };
        let mut parts: [Option<Vec<u8>>; 4] = Default::default();
        for part in &mut parts {
            if d.flag()? {
                *part = Some(layers::bytes(d)?);
            }
        }
        let [roster, state, graph, capitulations] = parts;
        self.roster = roster.as_deref().map(Roster::decode).transpose()?;
        self.state = state.as_deref().map(State::decode).transpose()?;
        self.graph = graph.as_deref().map(layers::decode_relations).transpose()?;
        self.capitulations = capitulations
            .as_deref()
            .map(layers::decode_capitulations)
            .transpose()?;
        self.named = layers::decode_names(&layers::bytes(d)?)?
            .into_iter()
            .collect();
        self.members = Records::load(d, Members::decode)?;
        self.labels = Records::load(d, Label::decode)?;
        for _ in 0..d.var()? {
            let identity = (layers::word(d)?, layers::word(d)?);
            let list = layers::decode_list(&layers::bytes(d)?)?;
            self.lists.insert(identity, list);
        }
        d.end().map_err(|_| saved_invalid())
    }

    /// What the emit goes on with after a cut: its state, the packets holding its tables, each
    /// once, its layers, then its carry.
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
        let layers = self.save_layers()?;
        out.extend_from_slice(&(layers.len() as u64).to_le_bytes());
        out.extend_from_slice(&layers);
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn load(directory: &Directory, bytes: &[u8]) -> io::Result<Self> {
        let mut emit = Self::new();
        emit.actors = Actors::resume(SCHEMA, directory)?;
        emit.family = Some(Keys::of(directory)?);
        let mut d = Decode::new(bytes);
        let length = usize::try_from(d.u64()?).map_err(|_| saved_invalid())?;
        let saved: Saved = serde_json::from_slice(d.bytes(length)?)?;
        let count = usize::try_from(d.u64()?).map_err(|_| saved_invalid())?;
        let mut packets = Vec::new();
        for _ in 0..count {
            let length = usize::try_from(d.u64()?).map_err(|_| saved_invalid())?;
            packets.push(Arc::new(d.bytes(length)?.to_vec()));
        }
        let length = usize::try_from(d.u64()?).map_err(|_| saved_invalid())?;
        emit.load_layers(d.bytes(length)?)?;
        let at = d.position();
        emit.carry = Carry::load(&bytes[at..])?;
        emit.last = saved.last;
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
    published: u32,
    next: Option<i32>,
    relations: Vec<[KeyHandle; 4]>,
    /// Each held table and the place of the packet holding it.
    held: Vec<(u16, u32)>,
}
