//! Wars facts, projected hour by hour from a columnar walk ([`replay::segments`]): the input
//! tables that changed in a field the wars compare ([`inputs::same_inputs`]), whose packed
//! views the segment's end hands on once; every input table's view at a segment's first hour,
//! which the ordered emit ([`super::emit`]) compares with what came before; the next captured
//! capitulation's date where it is read again; what the hour changed of the prepared layers
//! ([`super::layers`]); the war relations' lifetimes and casualty counters, whole where the
//! relations are joined again and else only those that changed; and the participation
//! tables' changed rows. An hour that changes none of it carries nothing but its hour.
//!
//! Tables whose readers read a few of their fields are held by those fields alone, so an hour
//! changing only others is not read. The relations are joined again only where a table the join
//! reads changed in a field it reads: the war overview's scores move every hour of a war, its
//! countries seldom. An hour holds no table's view past its segment's first: the emit keeps what
//! it published as packets.

use super::inputs::{self, INPUTS, JOIN, PARTICIPATION, PartRef};
use super::layers::{Layered, Layers};
use super::*;
use dashboard_world::{Hold, Needs, World as Shared};
use replay::{
    columnar_frames::view_heap_bytes,
    columns,
    segments::{self, Operator, Sink},
};
use replay_data::{format::FormatVersion, rows::RowCodec as _};
use replay_frame::frame::{SharedView, not_selected};
use std::mem::size_of;

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WarRelations,
    TableId::WarOwnedRelations,
    TableId::WarRelationGoals,
    TableId::WarOverviewCountries,
    TableId::WarNameRules,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::LocalPlayer,
    TableId::Capitulation,
    TableId::LexicalTokens,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::States,
    TableId::WarParticipation,
    TableId::WarLendLeaseHistories,
];

/// The fields the wars' readers read of a table they do not read whole.
fn compared(table: TableId) -> &'static [&'static str] {
    match table {
        TableId::Countries => &["canonical_slot", "tag"],
        TableId::States => &["custom_name", "owner_name"],
        TableId::Politics => &["ruling_party"],
        TableId::PoliticalParties => &["token"],
        TableId::Autonomy => &["state"],
        TableId::CountryAppearance => &["cosmetic_tag", "name_tag", "leader_ideology", "overlord"],
        TableId::Capitulation => &["capitulated", "capitulated_hour"],
        _ => &[],
    }
}

/// The fields of an input table whose change joins the relations again; none for a table the
/// join does not read. Of the war overview's scores the join reads only which countries they
/// are, their keys: their rows and status alone join again.
fn rejoined(table: TableId) -> Option<u64> {
    match table {
        _ if !JOIN.contains(&table) => None,
        TableId::WarOverviewCountries => Some(0),
        _ => Some(u64::MAX),
    }
}

fn mask(table: TableId, names: &[&str]) -> io::Result<u64> {
    names
        .iter()
        .try_fold(0, |mask, name| Ok(mask | 1 << columns::field(table, name)?))
}

/// How the walk's frame holds each table a projection reads.
fn holds() -> io::Result<Vec<(TableId, Hold)>> {
    TABLES
        .iter()
        .map(|&table| {
            let names = compared(table);
            Ok((
                table,
                if names.is_empty() {
                    Hold::Whole
                } else {
                    Hold::Fields(mask(table, names)?).of(table)
                },
            ))
        })
        .collect()
}

/// What a projection reads of its walk's world: its tables, in the shared frame.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        frame: holds()?,
        ..Needs::default()
    })
}

/// The war relations an hour joined, or the counters that changed since.
#[derive(replay_spool::Spooled)]
pub enum Relations {
    /// Every relation's lifetime and counters, in WarRelations row order; none when the hour's
    /// inputs could not be joined.
    Whole(Vec<(WarRelationKey, [i64; 4])>),
    /// The counters of the relations at these places in the last whole list that changed.
    Values(Vec<(u32, [i64; 4])>),
}

/// What the ordered emit reads of an hour that changes what the wars read.
#[derive(replay_spool::Spooled)]
pub struct Facts {
    /// Every input table's view, at a segment's first hour.
    pub whole: Option<Vec<(TableId, SharedView)>>,
    /// The input tables that changed since the hour before, one bit by [`INPUTS`] place.
    pub changed: u32,
    /// The first captured capitulation date after the hour, where the hour read the
    /// capitulations again or reached the date read before.
    pub next: Option<Option<i32>>,
    pub relations: Option<Relations>,
    /// The participation tables' packed parts, and whether they are whole.
    pub parts: Option<(Vec<u8>, bool)>,
    /// What the hour changed of the prepared layers.
    pub layers: Option<Box<Layered>>,
}

/// One hour of a segment, or its end.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Whether the walk walked the hour before in this segment: a segment's first hour and one
    /// after an hour not walked do not.
    pub follows: bool,
    pub facts: Option<Box<Facts>>,
    /// The segment's end, after its last hour `hour`: the input tables that changed since its
    /// first hour, by [`INPUTS`] place, and their last views packed ([`inputs::pack`]).
    pub ended: Option<(u32, Vec<u8>)>,
}

/// A relation the projector last joined: its lifetime, whether its row's endpoints are reversed
/// in it, and the counters it sent.
struct Joined {
    key: WarRelationKey,
    reversed: bool,
    sent: [i64; 4],
}

/// WarRelations' casualty counters: first and second casualties, then their unknown casualties.
const COUNTER_FIELDS: [&str; 4] = [
    "first_casualties",
    "second_casualties",
    "first_unknown_casualties",
    "second_unknown_casualties",
];

/// Each joined relation's place by its WarRelations row's key.
type Places = foldhash::HashMap<u128, u32>;

const SAVED: &[u8] = b"wars-projector-3";

/// The segment's projector.
pub struct Projector {
    format: FormatVersion,
    /// The content the layers name wars and countries with; none without the game's content.
    cat: Option<Arc<Catalogue>>,
    /// The layers this segment computed so far.
    layers: Layers,
    /// How the walk's frame is asked for each input table, by [`INPUTS`] place.
    inputs: Vec<(TableId, Hold)>,
    /// The fields of each input table whose change it reads again, by [`INPUTS`] place.
    watched: Vec<u64>,
    /// The fields of each input table whose change joins the relations again, by [`INPUTS`]
    /// place ([`rejoined`]).
    rejoins: Vec<Option<u64>>,
    /// WarRelations' casualty counters, and their columns: first and second casualties, then
    /// their unknown casualties.
    counters: u64,
    fields: [usize; 4],
    /// Capitulation's [`INPUTS`] place.
    capitulation: usize,
    /// The input tables changed since the segment's first hour, by [`INPUTS`] place.
    pending: u32,
    cold: bool,
    /// The branch last walked.
    branch: u64,
    last: Option<i32>,
    /// Each input table's view as last compared, by [`INPUTS`] place; WarRelations' without
    /// its counters.
    held: Vec<Option<SharedView>>,
    /// The first captured capitulation date after the hour it was last read at.
    next: Option<i32>,
    /// The relations last joined, in row order, and each one's place by its row's key; none
    /// while the inputs could not be joined.
    joined: Option<Vec<Joined>>,
    places: Places,
    /// Participation hours sent as diffs since they were last whole.
    chain: u32,
    /// The frame the walk's views are asked into, kept from hour to hour, and the hour's asks
    /// of the input, join and participation tables.
    frame: DecodedFrame,
    /// The input tables as last compared, in a frame of their own the layers read, kept from
    /// hour to hour.
    layered: DecodedFrame,
    asks: Vec<(TableId, Hold)>,
    join: Vec<(TableId, Hold)>,
    wholes: Vec<(TableId, Hold)>,
    /// A participation table's written slots, and what is packed before it is compressed,
    /// kept from hour to hour.
    slots: Vec<u32>,
    scratch: Vec<u8>,
}

/// The relations `live`, a frame of the join tables, joins: each one's lifetime and counters,
/// and what the projector keeps of them, with each one's place by its row's key.
fn join(live: &DecodedFrame) -> (Relations, Option<Vec<Joined>>, Places) {
    let first = live
        .complete::<tables::Countries>()
        .ok()
        .and_then(|rows| rows.first())
        .and_then(|row| std::str::from_utf8(&row.tag).ok());
    let Ok(read) = super::Hour::read(live, first) else {
        return (Relations::Whole(Vec::new()), None, Default::default());
    };
    // Joined relations follow WarRelations' row order.
    let places = live
        .complete::<tables::WarRelations>()
        .map(|rows| {
            rows.iter()
                .enumerate()
                .map(|(at, row)| (row.key(), at as u32))
                .collect()
        })
        .unwrap_or_default();
    let mut joined = Vec::with_capacity(read.relation_order.len());
    let mut list = Vec::with_capacity(read.relation_order.len());
    for pair in &read.relation_order {
        let row = &read.world.relations[pair];
        let key = WarRelationKey::new(
            read.identities[&row.first_country],
            read.identities[&row.second_country],
            row.start_hour,
        );
        let reversed = read.identities[&row.first_country] != key.first;
        let sent = inputs::counters(row, reversed);
        list.push((key.clone(), sent));
        joined.push(Joined {
            key,
            reversed,
            sent,
        });
    }
    (Relations::Whole(list), Some(joined), places)
}

/// The view of `table` an input holds: WarRelations' without its counters.
fn held_view(table: TableId, view: &SharedView) -> SharedView {
    if table == TableId::WarRelations {
        inputs::zeroed(view)
    } else {
        view.clone()
    }
}

impl Projector {
    pub fn new(format: FormatVersion, cat: Option<Arc<Catalogue>>) -> io::Result<Self> {
        let holds = holds()?;
        let hold = |table: TableId| {
            holds
                .iter()
                .copied()
                .find(|&(held, _)| held == table)
                .unwrap_or((table, Hold::Whole))
        };
        let structure = mask(
            TableId::WarRelations,
            &["start_hour", "end_hour", "cancel", "first_was_instigator"],
        )?;
        let watched = INPUTS
            .iter()
            .map(|&table| match hold(table) {
                (_, Hold::Fields(fields)) => fields,
                // Counters alone are the relations' values, read from their rows.
                _ if table == TableId::WarRelations => structure,
                _ => u64::MAX,
            })
            .collect();
        Ok(Self {
            format,
            layers: Layers::new(cat.clone()),
            cat,
            inputs: INPUTS.iter().map(|&table| hold(table)).collect(),
            watched,
            rejoins: INPUTS.iter().map(|&table| rejoined(table)).collect(),
            counters: mask(TableId::WarRelations, &COUNTER_FIELDS)?,
            fields: [
                columns::field(TableId::WarRelations, COUNTER_FIELDS[0])?,
                columns::field(TableId::WarRelations, COUNTER_FIELDS[1])?,
                columns::field(TableId::WarRelations, COUNTER_FIELDS[2])?,
                columns::field(TableId::WarRelations, COUNTER_FIELDS[3])?,
            ],
            capitulation: inputs::input(TableId::Capitulation)
                .ok_or_else(|| invalid("Capitulation is not a wars input"))?,
            pending: 0,
            cold: true,
            branch: 0,
            last: None,
            held: vec![None; INPUTS.len()],
            next: None,
            joined: None,
            places: Default::default(),
            chain: 0,
            frame: DecodedFrame {
                branch: 0,
                hour: 0,
                metrics: Default::default(),
                tables: std::array::from_fn(|_| not_selected()),
            },
            layered: DecodedFrame {
                branch: 0,
                hour: 0,
                metrics: Default::default(),
                tables: std::array::from_fn(|_| not_selected()),
            },
            asks: Vec::with_capacity(INPUTS.len()),
            join: JOIN.iter().map(|&table| hold(table)).collect(),
            wholes: Vec::with_capacity(PARTICIPATION.len()),
            slots: Vec::new(),
            scratch: Vec::new(),
        })
    }

    /// The counters of the joined relations whose rows the hour wrote and that changed since
    /// they were sent, read from those rows alone; None when the hour inserted or deleted rows or
    /// read the table in part, which joins the relations again.
    fn moved(&mut self, state: &segments::State) -> io::Result<Option<Vec<(u32, [i64; 4])>>> {
        let table = TableId::WarRelations;
        let changes = columns::changes(state, table)?;
        if changes.truncated || !changes.inserted.is_empty() || !changes.deleted.is_empty() {
            return Ok(None);
        }
        let Some(joined) = self.joined.as_mut() else {
            return Ok(Some(Vec::new()));
        };
        let held = columns::table(state, table)?;
        let mut moved = Vec::new();
        let written = changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks));
        for (&slot, &mask) in written {
            if mask & self.counters == 0 {
                continue;
            }
            let Some(key) = held.key(slot) else {
                continue;
            };
            let Some(&at) = self.places.get(&key) else {
                return Ok(None);
            };
            // First and second casualties, then their unknown casualties, as raw counters.
            let mut raw = [0_i64; 4];
            let mut decoded: Option<rows::WarRelationRow> = None;
            for (index, (value, &field)) in raw.iter_mut().zip(&self.fields).enumerate() {
                *value = if held.recorded(field, slot) {
                    columns::plain::<i64>(held, field)?[slot as usize]
                } else {
                    if decoded.is_none() {
                        decoded = Some(columns::row(state, table, slot, self.format.version())?);
                    }
                    let row = decoded.as_ref().expect("the row was decoded");
                    [
                        row.first_casualties,
                        row.second_casualties,
                        row.first_unknown_casualties,
                        row.second_unknown_casualties,
                    ][index]
                };
            }
            let row = rows::WarRelationRow {
                first_casualties: raw[0],
                second_casualties: raw[1],
                first_unknown_casualties: raw[2],
                second_unknown_casualties: raw[3],
                ..Default::default()
            };
            let held = &mut joined[at as usize];
            let now = inputs::counters(&row, held.reversed);
            if now != held.sent {
                held.sent = now;
                moved.push((at, now));
            }
        }
        Ok(Some(moved))
    }

    /// The participation parts of the hour, packed: both tables whole when `whole`, else each
    /// one's changed rows where it changed. The views it asks for stay in the frame until the
    /// hour releases them.
    fn parts(
        &mut self,
        hour: &segments::Hour<'_, Shared>,
        whole: bool,
    ) -> io::Result<Option<(Vec<u8>, bool)>> {
        let (state, world) = (hour.state, hour.world);
        let whole = whole || self.chain >= inputs::WHOLE_EVERY;
        let mut diffs: [Option<(rows::TableRows, &[u128])>; 2] = [None, None];
        self.wholes.clear();
        for (at, &table) in PARTICIPATION.iter().enumerate() {
            if whole {
                self.wholes.push((table, Hold::Whole));
                continue;
            }
            if !world.touched(table, u64::MAX) {
                continue;
            }
            let changes = columns::changes(state, table)?;
            if world.restated(table) || !columns::complete(state, table) || changes.truncated {
                self.wholes.push((table, Hold::Whole));
                continue;
            }
            let held = columns::table(state, table)?;
            self.slots.clear();
            self.slots.extend(
                changes
                    .changed
                    .iter()
                    .chain(&changes.fill_slots)
                    .chain(&changes.inserted)
                    .copied()
                    .filter(|&slot| held.key(slot).is_some()),
            );
            let upserts = columns::rows_at(state, table, &self.slots, self.format.version())?;
            diffs[at] = Some((upserts, &changes.deleted_keys[..]));
        }
        if !self.wholes.is_empty() {
            world.copy_into(state, &self.wholes, &mut self.frame)?;
        }
        let mut parts: [Option<PartRef<'_>>; 2] = [None, None];
        for ((part, diff), &table) in parts.iter_mut().zip(&diffs).zip(&PARTICIPATION) {
            if self.wholes.iter().any(|&(asked, _)| asked == table) {
                *part = Some(PartRef::Whole(&self.frame.tables[table.index()]));
            } else if let Some((upserts, deleted)) = diff {
                *part = Some(PartRef::Diff {
                    upserts,
                    deleted: *deleted,
                });
            }
        }
        if parts.iter().all(Option::is_none) {
            return Ok(None);
        }
        let complete = parts
            .iter()
            .all(|part| matches!(part, Some(PartRef::Whole(_))));
        let packed = inputs::pack_parts(&parts, &mut self.scratch)?;
        self.chain = if complete { 0 } else { self.chain + 1 };
        Ok(Some((packed, complete)))
    }

    fn heap(&self) -> usize {
        self.held
            .iter()
            .flatten()
            .map(|view| view_heap_bytes(view))
            .sum::<usize>()
            + self
                .joined
                .as_ref()
                .map_or(0, |joined| joined.capacity() * size_of::<Joined>())
            + self.places.capacity() * size_of::<(u128, u32)>()
            + self.slots.capacity() * size_of::<u32>()
            + self.scratch.capacity()
    }
}

// Every segment starts cold: its first hour reads every table again; what came before is the
// emit's.
impl Operator<Shared> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, Shared>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("wars.columnar", Some("wars"));
        let (at, state, world) = (hour.hour, hour.state, hour.world);
        let whole = self.cold || hour.starts;
        let follows = !whole && self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if whole {
            self.layers = Layers::new(self.cat.clone());
        }
        self.cold = false;
        self.last = Some(at);
        self.asks.clear();
        let mut asked = 0_u32;
        for (place, (&table, &fields)) in INPUTS.iter().zip(&self.watched).enumerate() {
            if whole || world.touched(table, fields) {
                self.asks.push(self.inputs[place]);
                asked |= 1 << place;
            }
        }
        if !self.asks.is_empty() {
            world.copy_into(state, &self.asks, &mut self.frame)?;
        }
        let mut whole_views = None;
        let mut changed = 0_u32;
        if whole {
            let views: Vec<_> = INPUTS
                .iter()
                .map(|&table| (table, held_view(table, &self.frame.tables[table.index()])))
                .collect();
            for (held, (_, view)) in self.held.iter_mut().zip(&views) {
                *held = Some(view.clone());
            }
            whole_views = Some(views);
        } else {
            for place in inputs::places(asked) {
                let table = INPUTS[place];
                let view = &self.frame.tables[table.index()];
                let same = self.held[place]
                    .as_ref()
                    .is_some_and(|held| inputs::same_inputs(table, held, view));
                if !same {
                    self.held[place] = Some(held_view(table, view));
                    changed |= 1 << place;
                }
            }
        }
        // The emit compares a first hour's views itself; what changes after waits for the end.
        self.pending = if whole { 0 } else { self.pending | changed };
        let layers = if !follows || changed != 0 {
            let tables = &mut self.layered;
            tables.branch = hour.branch;
            tables.hour = at;
            for (view, &table) in self.held.iter().zip(INPUTS) {
                tables.tables[table.index()] = view.clone().unwrap_or_else(not_selected);
            }
            self.layers
                .hour(&self.layered, !follows, changed)
                .map(Box::new)
        } else {
            None
        };
        // A capitulation read before becomes usable at its date.
        let next = if whole
            || asked & 1 << self.capitulation != 0
            || self.next.is_some_and(|next| next <= at)
        {
            self.next = self.held[self.capitulation]
                .as_ref()
                .and_then(|view| inputs::next_capitulation(view, at));
            Some(self.next)
        } else {
            None
        };
        let mut rejoin = !follows
            || inputs::places(changed).any(|place| {
                self.rejoins[place].is_some_and(|fields| world.touched(INPUTS[place], fields))
            });
        let mut relations = None;
        if !rejoin && world.touched(TableId::WarRelations, self.counters) {
            match self.moved(state)? {
                Some(moved) if moved.is_empty() => {}
                Some(moved) => relations = Some(Relations::Values(moved)),
                None => rejoin = true,
            }
        }
        if rejoin {
            world.copy_into(state, &self.join, &mut self.frame)?;
            let (whole, joined, places) = join(&self.frame);
            relations = Some(whole);
            self.joined = joined;
            self.places = places;
            dashboard_world::release(&mut self.frame, &self.join);
        }
        dashboard_world::release(&mut self.frame, &self.asks);
        let parts = self.parts(hour, !follows);
        dashboard_world::release(&mut self.frame, &self.wholes);
        let parts = parts?;
        let quiet = whole_views.is_none()
            && changed == 0
            && next.is_none()
            && relations.is_none()
            && parts.is_none()
            && layers.is_none();
        self.branch = hour.branch;
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            follows,
            facts: (!quiet).then(|| {
                Box::new(Facts {
                    whole: whole_views,
                    changed,
                    next,
                    relations,
                    parts,
                    layers,
                })
            }),
            ended: None,
        })
    }

    /// Hands on the views of the tables changed since the segment's first hour, once: the emit
    /// reads its held tables only at a segment's first hour and as it saves, both past here.
    fn end(&mut self, out: &mut dyn Sink<Hour>) -> io::Result<()> {
        let (pending, Some(last)) = (std::mem::take(&mut self.pending), self.last) else {
            return Ok(());
        };
        if pending == 0 {
            return Ok(());
        }
        let views = inputs::places(pending).map(|place| {
            let view = self.held[place].as_ref().expect("a changed table is held");
            (INPUTS[place], view)
        });
        let packet = inputs::pack(views, &mut self.scratch)?;
        out.push(Hour {
            branch: self.branch,
            hour: last,
            follows: false,
            facts: None,
            ended: Some((pending, packet)),
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(self.heap())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved wars projector",
            ));
        }
        *self = Self::new(self.format, self.cat.clone())?;
        Ok(())
    }
}
