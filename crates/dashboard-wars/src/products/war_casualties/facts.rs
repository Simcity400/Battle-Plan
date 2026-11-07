//! Casualty facts, projected hour by hour from a columnar walk ([`replay::segments`]). An hour
//! the scan joins anew is carried whole: a segment's first hour, one after an hour not walked or
//! not joined, and one changing the campaign, the census (countries' slots and tags, every
//! link) or the relation lifetimes (a row inserted or deleted, a start moved, the table read in
//! part or restated). Every other hour carries only the relations whose casualty counters
//! changed, by their place in the rows last carried whole, and the naming tables' views where a
//! naming field was written. An hour changing none of it carries nothing but its hour.

use super::{NAME_INPUTS, NAME_TABLES, SCAN_TABLES, joins};
use dashboard_world::{Hold, Needs, World as Shared};
use replay::{
    columns,
    campaign::Replay,
    segments::{self, Operator, Sink},
};
use replay_data::{
    format::FormatVersion,
    rows::{self, TableId, tables},
};
use replay_frame::frame::{DecodedFrame, Metrics, SharedView, not_selected};
use std::{io, mem::size_of};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &SCAN_TABLES;

/// The fields the scan reads of a table it does not read whole.
fn compared(table: TableId) -> &'static [&'static str] {
    match table {
        TableId::Countries => &["canonical_slot", "tag"],
        TableId::CountryAppearance => &["cosmetic_tag", "name_tag", "leader_ideology", "overlord"],
        TableId::Politics => &["ruling_party"],
        TableId::PoliticalParties => &["token"],
        TableId::Autonomy => &["state"],
        _ => &[],
    }
}

fn mask(table: TableId, names: &[&str]) -> io::Result<u64> {
    names
        .iter()
        .try_fold(0, |mask, name| Ok(mask | 1 << columns::field(table, name)?))
}

/// How the walk's frame holds `table`.
fn hold(table: TableId) -> io::Result<(TableId, Hold)> {
    let names = compared(table);
    Ok((
        table,
        if names.is_empty() {
            Hold::Whole
        } else {
            Hold::Fields(mask(table, names)?).of(table)
        },
    ))
}

/// What a projection reads of its walk's world: its tables, in the shared frame.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        frame: TABLES
            .iter()
            .map(|&table| hold(table))
            .collect::<io::Result<_>>()?,
        ..Needs::default()
    })
}

/// What the ordered emit reads of an hour that changes what the scan reads.
#[derive(replay_spool::Spooled)]
pub enum Facts {
    /// Every table the scan reads, and the hour's metrics.
    Whole {
        tables: Vec<(TableId, SharedView)>,
        metrics: Box<Metrics>,
    },
    /// The relations whose counters changed, by their place in the rows last carried whole, and
    /// their counters as those rows order them; and the naming tables' views and the hour's
    /// metrics, where a naming field was written.
    Moved {
        relations: Vec<(u32, [i64; 4])>,
        names: Option<(Vec<(TableId, SharedView)>, Box<Metrics>)>,
    },
}

/// One hour of a segment.
#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub facts: Option<Box<Facts>>,
}

/// WarRelations' casualty counters: first and second casualties, then their unknown casualties.
const COUNTER_FIELDS: [&str; 4] = [
    "first_casualties",
    "second_casualties",
    "first_unknown_casualties",
    "second_unknown_casualties",
];

const SAVED: &[u8] = b"casualties-projector-1";

/// The segment's projector.
pub struct Projector {
    format: FormatVersion,
    /// The recording's campaign, which every hour joined holds.
    campaign: Vec<u8>,
    /// How the frame is asked for every table, and for the naming tables.
    asks: Vec<(TableId, Hold)>,
    names: Vec<(TableId, Hold)>,
    /// The census fields of Countries, and the compared fields of each naming input.
    census: u64,
    naming: Vec<(TableId, u64)>,
    /// WarRelations' casualty counters and their columns, and its start field.
    counters: u64,
    fields: [usize; 4],
    start: u64,
    cold: bool,
    last: Option<i32>,
    /// Whether the hour projected last was joined, which the next goes on from by its changes.
    joined: bool,
    /// Each relation's place in the rows last carried whole, by its row's key, and its counters
    /// as last carried.
    places: foldhash::HashMap<u128, u32>,
    sent: Vec<[i64; 4]>,
    /// The frame the walk's views are asked into, kept from hour to hour.
    frame: DecodedFrame,
}

fn counters(row: &rows::WarRelationRow) -> [i64; 4] {
    [
        row.first_casualties,
        row.second_casualties,
        row.first_unknown_casualties,
        row.second_unknown_casualties,
    ]
}

impl Projector {
    pub fn new(replay: &Replay) -> io::Result<Self> {
        let field = |name| columns::field(TableId::WarRelations, name);
        Ok(Self {
            format: replay.format,
            campaign: replay.campaign_id.clone(),
            asks: TABLES
                .iter()
                .map(|&table| hold(table))
                .collect::<io::Result<_>>()?,
            names: NAME_TABLES
                .iter()
                .map(|&table| hold(table))
                .collect::<io::Result<_>>()?,
            census: mask(TableId::Countries, compared(TableId::Countries))?,
            naming: NAME_INPUTS
                .iter()
                .map(|&table| Ok((table, mask(table, compared(table))?)))
                .collect::<io::Result<_>>()?,
            counters: mask(TableId::WarRelations, &COUNTER_FIELDS)?,
            fields: [
                field(COUNTER_FIELDS[0])?,
                field(COUNTER_FIELDS[1])?,
                field(COUNTER_FIELDS[2])?,
                field(COUNTER_FIELDS[3])?,
            ],
            start: mask(TableId::WarRelations, &["start_hour"])?,
            cold: true,
            last: None,
            joined: false,
            places: Default::default(),
            sent: Vec::new(),
            frame: DecodedFrame {
                branch: 0,
                hour: 0,
                metrics: Default::default(),
                tables: std::array::from_fn(|_| not_selected()),
            },
        })
    }

    /// The relations whose counters the hour changed, by place, as their rows order them; None
    /// when the hour inserted, deleted or moved the start of a relation, read the table in part
    /// or restated it, or wrote a row not carried whole, which joins the relations anew.
    fn moved(
        &mut self,
        state: &segments::State,
        world: &Shared,
    ) -> io::Result<Option<Vec<(u32, [i64; 4])>>> {
        let table = TableId::WarRelations;
        if !world.touched(table, self.counters | self.start) {
            return Ok(Some(Vec::new()));
        }
        let changes = columns::changes(state, table)?;
        if world.restated(table)
            || !columns::complete(state, table)
            || changes.truncated
            || !changes.inserted.is_empty()
            || !changes.deleted.is_empty()
        {
            return Ok(None);
        }
        let held = columns::table(state, table)?;
        let mut moved = Vec::new();
        let written = changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks));
        for (&slot, &mask) in written {
            if mask & self.start != 0 {
                return Ok(None);
            }
            if mask & self.counters == 0 {
                continue;
            }
            let Some(&at) = held.key(slot).and_then(|key| self.places.get(&key)) else {
                return Ok(None);
            };
            let mut now = [0_i64; 4];
            let mut decoded: Option<rows::WarRelationRow> = None;
            for (index, (value, &field)) in now.iter_mut().zip(&self.fields).enumerate() {
                *value = if held.recorded(field, slot) {
                    columns::plain::<i64>(held, field)?[slot as usize]
                } else {
                    // An unrecorded cell reads as the row decoder reads it.
                    if decoded.is_none() {
                        decoded = Some(columns::row(state, table, slot, self.format.version())?);
                    }
                    counters(decoded.as_ref().expect("the row was decoded"))[index]
                };
            }
            let sent = &mut self.sent[at as usize];
            if now != *sent {
                *sent = now;
                moved.push((at, now));
            }
        }
        moved.sort_unstable_by_key(|&(at, _)| at);
        Ok(Some(moved))
    }

    /// Copies `asks` of the walk's views into the frame, in the fields each is held by, with the
    /// hour's metrics.
    fn copy(
        &mut self,
        state: &segments::State,
        world: &Shared,
        asks: &[(TableId, Hold)],
    ) -> io::Result<()> {
        self.frame.metrics.clone_from(world.metrics());
        for &(table, hold) in asks {
            self.frame.tables[table.index()] = world.copy(state, table, hold)?;
        }
        Ok(())
    }

    /// The views of `asks` the frame holds, and the hour's metrics.
    fn views(&self, asks: &[(TableId, Hold)]) -> (Vec<(TableId, SharedView)>, Box<Metrics>) {
        let views = asks
            .iter()
            .map(|&(table, _)| (table, self.frame.tables[table.index()].clone()))
            .collect();
        (views, Box::new(self.frame.metrics.clone()))
    }

    /// The hour carried whole: every table's view, and the places and counters of the relations
    /// it joins.
    fn whole(&mut self, state: &segments::State, world: &Shared) -> io::Result<Facts> {
        let asks = std::mem::take(&mut self.asks);
        let copied = self.copy(state, world, &asks);
        self.asks = asks;
        copied?;
        self.places.clear();
        self.sent.clear();
        self.joined = joins(&self.frame, &self.campaign);
        if self.joined
            && let Ok(rows) = self.frame.complete::<tables::WarRelations>()
        {
            for (place, row) in rows.iter().enumerate() {
                self.places.insert(row.key(), place as u32);
                self.sent.push(counters(row));
            }
        }
        let (tables, metrics) = self.views(&self.asks);
        dashboard_world::release(&mut self.frame, &self.asks);
        Ok(Facts::Whole { tables, metrics })
    }

    fn heap(&self) -> usize {
        self.places.capacity() * size_of::<(u128, u32)>()
            + self.sent.capacity() * size_of::<[i64; 4]>()
            + self.campaign.capacity()
    }
}

// Every segment starts cold: its first hour is carried whole; what came before is the emit's.
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
        let _cpu = replay_frame::cpu::scope("casualties.columnar", Some("casualties"));
        let (at, state, world) = (hour.hour, hour.state, hour.world);
        let follows = !self.cold
            && !hour.starts
            && self.last.and_then(|last| last.checked_add(1)) == Some(at);
        self.cold = false;
        self.last = Some(at);
        let mut moved = None;
        if follows
            && self.joined
            && !world.touched(TableId::Campaign, u64::MAX)
            && !world.touched(TableId::Countries, self.census)
            && !world.touched(TableId::CountryLinks, u64::MAX)
        {
            moved = self.moved(state, world)?;
        }
        let Some(relations) = moved else {
            let facts = self.whole(state, world)?;
            return out.push(Hour {
                branch: hour.branch,
                hour: at,
                facts: Some(Box::new(facts)),
            });
        };
        let names = if self
            .naming
            .iter()
            .any(|&(table, fields)| world.touched(table, fields))
        {
            let names = std::mem::take(&mut self.names);
            let copied = self.copy(state, world, &names);
            self.names = names;
            copied?;
            let views = self.views(&self.names);
            dashboard_world::release(&mut self.frame, &self.names);
            Some(views)
        } else {
            None
        };
        let quiet = relations.is_empty() && names.is_none();
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            facts: (!quiet).then(|| Box::new(Facts::Moved { relations, names })),
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
                "Not a saved casualties projector",
            ));
        }
        self.cold = true;
        self.last = None;
        self.joined = false;
        self.places.clear();
        self.sent.clear();
        Ok(())
    }
}
