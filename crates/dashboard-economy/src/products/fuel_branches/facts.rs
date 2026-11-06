//! Fuel by branch's per-segment operator: the consumer cells each hour changed.

use dashboard_world::{Needs, Roster, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{CountryFuelConsumerRow, TableId},
    scope::CaptureMode,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

pub const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryFuelConsumers,
];
const CONSUMERS: TableId = TableId::CountryFuelConsumers;

pub fn needs() -> Needs {
    Needs {
        countries: true,
        ..Needs::default()
    }
}

pub type Values = [i64; 3];

pub type Cell = (u32, u32);

#[derive(replay_spool::Spooled)]
pub struct Facts {
    pub whole: bool,
    pub cells: Vec<(Cell, Option<Values>)>,
    pub unidentified: u32,
    pub tags: Vec<(u32, Option<Arc<str>>)>,
}

#[derive(replay_spool::Spooled)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    pub observed: bool,
    pub read: bool,
    pub refusal: Option<Box<str>>,
    pub facts: Option<Box<Facts>>,
}

#[derive(Clone, Copy)]
struct Row {
    raw: u32,
    index: u32,
    values: Values,
    slot: Option<u32>,
}

fn refused(
    state: &State,
    table: TableId,
    status: columns::Status,
    mode: CaptureMode,
    hour: i32,
) -> io::Result<String> {
    Ok(match status {
        columns::Status::Unavailable { reason } => {
            let reason = state
                .dictionary()
                .get(reason)
                .ok_or_else(|| io::Error::other("A refused table's reason names no dictionary entry"))?;
            format!(
                "hour {hour} {table:?} is unavailable: {}",
                String::from_utf8_lossy(reason)
            )
        }
        columns::Status::Truncated { missing } => {
            format!("hour {hour} {table:?} is truncated ({missing} missing rows)")
        }
        columns::Status::NotRecorded => {
            format!("hour {hour} {table:?} is not recorded in a {mode} recording")
        }
        columns::Status::Complete => {
            return Err(io::Error::other("A complete table has no refusal"));
        }
    })
}

fn touched(state: &State, table: TableId) -> io::Result<bool> {
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty())
}

fn tag_of(roster: &Roster, slot: u32) -> Option<Arc<str>> {
    let country = roster.at_slot(slot).last()?;
    std::str::from_utf8(roster.tag(country.tag))
        .ok()
        .filter(|tag| !tag.is_empty())
        .map(Arc::from)
}

pub struct Projector {
    format: FormatVersion,
    mode: CaptureMode,
    last: Option<i32>,
    shapes: [Option<columns::Status>; 3],
    stale: bool,
    rows: HashMap<u128, Row>,
    /// The last raw id's row is the cell's, as the census keeps the last row in key order.
    cells: BTreeMap<Cell, Vec<u32>>,
    seen: HashMap<u32, u32>,
    unidentified: u32,
}

/// A saved projector restarts: everything it holds is read again from the walker.
const SAVED: &[u8] = b"fuel branches projector: cold";

impl Projector {
    pub fn new(format: FormatVersion, mode: CaptureMode) -> Self {
        Self {
            format,
            mode,
            last: None,
            shapes: [None; 3],
            stale: true,
            rows: HashMap::new(),
            cells: BTreeMap::new(),
            seen: HashMap::new(),
            unidentified: 0,
        }
    }

    fn clear(&mut self) {
        self.stale = true;
        self.rows.clear();
        self.cells.clear();
        self.seen.clear();
        self.unidentified = 0;
    }

    fn decode(&self, state: &State, slot: u32, roster: &Roster) -> io::Result<(u128, Row)> {
        let key = columns::table(state, CONSUMERS)?
            .key(slot)
            .ok_or_else(|| io::Error::other("A consumer slot holds no row"))?;
        let row: CountryFuelConsumerRow =
            columns::row(state, CONSUMERS, slot, self.format.version())?;
        Ok((
            key,
            Row {
                raw: row.country,
                index: row.index,
                values: [row.requested_raw, row.received_raw, i64::from(row.priority)],
                slot: roster.slot_of(row.country),
            },
        ))
    }

    fn remove(&mut self, key: u128, dirty: &mut BTreeSet<Cell>) {
        let Some(row) = self.rows.remove(&key) else {
            return;
        };
        let Some(slot) = row.slot else {
            self.unidentified -= 1;
            return;
        };
        let cell = (slot, row.index);
        if let Some(raws) = self.cells.get_mut(&cell) {
            if let Ok(at) = raws.binary_search(&row.raw) {
                raws.remove(at);
            }
            if raws.is_empty() {
                self.cells.remove(&cell);
            }
        }
        if let Some(count) = self.seen.get_mut(&slot) {
            *count -= 1;
            if *count == 0 {
                self.seen.remove(&slot);
            }
        }
        dirty.insert(cell);
    }

    fn insert(
        &mut self,
        key: u128,
        row: Row,
        dirty: &mut BTreeSet<Cell>,
        newly: &mut BTreeSet<u32>,
    ) {
        self.remove(key, dirty);
        match row.slot {
            Some(slot) => {
                let raws = self.cells.entry((slot, row.index)).or_default();
                if let Err(at) = raws.binary_search(&row.raw) {
                    raws.insert(at, row.raw);
                }
                let count = self.seen.entry(slot).or_default();
                if *count == 0 {
                    newly.insert(slot);
                }
                *count += 1;
                dirty.insert((slot, row.index));
            }
            None => self.unidentified += 1,
        }
        self.rows.insert(key, row);
    }

    fn value(&self, cell: Cell) -> Option<Values> {
        let raw = *self.cells.get(&cell)?.last()?;
        let key = (u128::from(raw) << 32) | u128::from(cell.1);
        self.rows.get(&key).map(|row| row.values)
    }

    fn rebuild(&mut self, state: &State, roster: &Roster) -> io::Result<()> {
        self.clear();
        self.stale = false;
        let live: Vec<u32> = columns::table(state, CONSUMERS)?.live().ones().collect();
        let (mut dirty, mut newly) = (BTreeSet::new(), BTreeSet::new());
        for slot in live {
            let (key, row) = self.decode(state, slot, roster)?;
            self.insert(key, row, &mut dirty, &mut newly);
        }
        Ok(())
    }

    fn update(
        &mut self,
        state: &State,
        roster: &Roster,
        dirty: &mut BTreeSet<Cell>,
        newly: &mut BTreeSet<u32>,
    ) -> io::Result<()> {
        if touched(state, CONSUMERS)? {
            let edits = columns::changes(state, CONSUMERS)?;
            for &key in &edits.deleted_keys {
                self.remove(key, dirty);
            }
            let mut slots: Vec<u32> = edits
                .changed
                .iter()
                .chain(&edits.fill_slots)
                .chain(&edits.inserted)
                .copied()
                .collect();
            slots.sort_unstable();
            slots.dedup();
            let held = columns::table(state, CONSUMERS)?;
            for slot in slots {
                if held.key(slot).is_some() {
                    let (key, row) = self.decode(state, slot, roster)?;
                    self.insert(key, row, dirty, newly);
                }
            }
        }
        let moved: HashSet<u32> = roster.changed_links().iter().copied().collect();
        if !moved.is_empty() {
            let relinked: Vec<(u128, Row)> = self
                .rows
                .iter()
                .filter(|(_, row)| moved.contains(&row.raw))
                .map(|(&key, &row)| (key, row))
                .collect();
            for (key, row) in relinked {
                let slot = roster.slot_of(row.raw);
                if slot != row.slot {
                    self.insert(key, Row { slot, ..row }, dirty, newly);
                }
            }
        }
        Ok(())
    }

    fn heap(&self) -> usize {
        self.rows.capacity() * size_of::<(u128, Row)>()
            + self.cells.len() * (size_of::<(Cell, Vec<u32>)>() + 2 * size_of::<u32>())
            + self.seen.capacity() * size_of::<(u32, u32)>()
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, start: segments::Start<'_>, _: u64, first: i32) -> io::Result<()> {
        self.clear();
        self.last = None;
        self.shapes = [None; 3];
        // A restart's tables are the hour before's, so its first hour's changes are the census's.
        if let segments::Start::Restart(state) = start {
            for (at, &table) in TABLES.iter().enumerate() {
                self.shapes[at] = Some(columns::status(state, table));
            }
            self.last = first.checked_sub(1);
        }
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("fuel_branches.columnar", Some("fuel_branches"));
        let state = hour.state;
        let at = hour.hour;
        let continuous = self.last.and_then(|last| last.checked_add(1)) == Some(at);
        if !continuous {
            self.shapes = [None; 3];
        }
        let restart = hour.starts || hour.opens || !continuous;
        self.last = Some(at);
        let mut statuses = [columns::Status::NotRecorded; 3];
        let mut shaped = restart;
        let mut observed = !continuous;
        for (at_table, &table) in TABLES.iter().enumerate() {
            let status = columns::status(state, table);
            statuses[at_table] = status;
            let moved = self.shapes[at_table] != Some(status);
            self.shapes[at_table] = Some(status);
            shaped |= moved;
            observed |= moved
                || (matches!(
                    status,
                    columns::Status::Complete | columns::Status::Truncated { .. }
                ) && touched(state, table)?);
        }
        let flags = metrics(hour.manifest);
        let index = CONSUMERS.index();
        let read = flags.table_ns[index] != 0
            || dashboard_text::metadata::slot_reads(self.format, &flags, at)[index];
        observed |= read;
        // The row census reads the countries before the consumers, and refuses the first.
        let refusal = TABLES
            .iter()
            .zip(statuses)
            .find(|&(_, status)| status != columns::Status::Complete);
        if let Some((&table, status)) = refusal {
            self.clear();
            let refusal = if observed {
                let what = if table == CONSUMERS {
                    "fuel consumers unavailable"
                } else {
                    "country identities unavailable"
                };
                let reason = refused(state, table, status, self.mode, at)?;
                Some(format!("{what}: {reason}").into_boxed_str())
            } else {
                None
            };
            return out.push(Hour {
                branch: hour.branch,
                hour: at,
                observed,
                read,
                refusal,
                facts: None,
            });
        }
        let roster = hour.world.countries()?;
        let whole = self.stale || shaped || roster.rebuilt();
        let before = self.unidentified;
        let (mut dirty, mut newly) = (BTreeSet::new(), BTreeSet::new());
        if whole {
            self.rebuild(state, roster)?;
        } else {
            self.update(state, roster, &mut dirty, &mut newly)?;
        }
        let renamed = whole || roster.changed();
        let facts = if whole || !dirty.is_empty() || !newly.is_empty() || renamed || before != self.unidentified
        {
            let cells = if whole {
                self.cells
                    .keys()
                    .map(|&cell| (cell, self.value(cell)))
                    .collect()
            } else {
                dirty
                    .into_iter()
                    .map(|cell| (cell, self.value(cell)))
                    .collect()
            };
            let mut slots: Vec<u32> = if renamed {
                self.seen.keys().copied().collect()
            } else {
                newly.into_iter().filter(|slot| self.seen.contains_key(slot)).collect()
            };
            slots.sort_unstable();
            let tags = slots
                .into_iter()
                .map(|slot| (slot, tag_of(roster, slot)))
                .collect();
            Some(Box::new(Facts {
                whole,
                cells,
                unidentified: self.unidentified,
                tags,
            }))
        } else {
            None
        };
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            observed,
            read,
            refusal: None,
            facts,
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
                "Not a saved fuel branches projector",
            ));
        }
        *self = Self::new(self.format, self.mode);
        Ok(())
    }
}
