//! A table's cells at an hour of a recording, read through the unit's reader primitives: from
//! its last restore at or before the hour ([`unit::reader::restore`]), then its parts advanced to
//! the hour in one call ([`Cursor::advance`]); later hours of the run stepped one at a time, each
//! step saying what it changed, or advanced to from where the cells stand unless a restore of the
//! table lies between. The cells are the reader's one copy of the table: a
//! [`TableState`] of the selected columns, with the strings its cells and status name.
//!
//! A log table's cells at an hour are the rows it lists then, read by ordinal at a seek; a step
//! hands its appends as inserted rows and its forgets as deleted ones.

use crate::{columnar_frames::{Damage, DamageScope, Recording}, rows::TableId};
use replay_columnar::{
    manifest::Status,
    state::Kind,
    store::{Strings, TableState, TableView, Value},
    unit::{
        self, BLOCK, TableEntry, lists,
        reader::{Advanced, Cursor, Event, Lives, LogCursor, LogEvent, Part},
        restore::RestoreEntry,
    },
};
use std::{collections::BTreeMap, io};

fn label(name: &str) -> engine_allocation::labels::Scope {
    engine_allocation::labels::enter(engine_allocation::labels::owner(name), engine_allocation::labels::Kind::WalkerTables)
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// What the hour a reader stepped to last changed.
#[derive(Debug, Default)]
pub struct Changes {
    /// The cells were read anew (a seek, a run's start or a pack): every row may have changed.
    pub anew: bool,
    /// Slots of rows the hour inserted, ascending.
    pub inserted: Vec<u32>,
    /// Each deleted row's slot and the key it held.
    pub deleted: Vec<(u32, u128)>,
    /// By selected column, the slots of rows older than the hour whose cell an event named.
    pub written: Vec<Vec<u32>>,
    /// The table's status, read flag or sample hour differs from the hour before.
    pub status_changed: bool,
}

impl Changes {
    /// Whether the hour changed nothing a reader of the table's rows sees.
    pub fn quiet(&self) -> bool {
        !self.anew && !self.status_changed && self.inserted.is_empty() && self.deleted.is_empty() && self.written.iter().all(Vec::is_empty)
    }

    fn clear(&mut self) {
        self.anew = false;
        self.inserted.clear();
        self.deleted.clear();
        self.written.iter_mut().for_each(Vec::clear);
        self.status_changed = false;
    }
}

/// One event of a selected column's track, as a stretch read whole gives it
/// ([`TableReader::read_stretch`]). A text cell's track is a number track of its string id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// A change from the cell's value before; a missing or unrecorded cell counts as 0.
    Difference(i64),
    Value(u64),
    Missing,
    Unrecorded,
    /// A list or opaque cell given a value, its bytes not carried.
    Written,
}

/// A track event: its selected column (by position among the selected), slot, the row's insert
/// hour when the part inserted it, the event's block hour, and the event.
#[derive(Clone, Copy, Debug)]
pub struct TrackEvent {
    pub column: usize,
    pub slot: u32,
    pub born: Option<u16>,
    pub hour: u16,
    pub change: Change,
}

/// A selected cell as a reader's cells hold it: a number's bits (sign-extended) or a text's
/// string id, missing, unrecorded, or a list or opaque cell holding a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    Bits(u64),
    Missing,
    Unrecorded,
    List,
}

impl Held {
    /// Cell `column` of `slot` in `view`.
    pub fn of(view: &TableView<'_>, column: usize, slot: u32) -> Self {
        match view.value(column, slot) {
            Value::Number(bits) => Self::Bits(bits),
            Value::Text(id) => Self::Bits(u64::from(id)),
            Value::Missing => Self::Missing,
            Value::Unrecorded => Self::Unrecorded,
            Value::List(_) => Self::List,
        }
    }
}

/// Block hours `from..=to` of one block, read with one advance: the rows live before `from`
/// with their selected cells, every track event of the selected columns in the stretch, each
/// insert and delete with its hour, each hour's status, and the rows each hour read.
#[derive(Debug, Default)]
pub struct Stretch {
    pub block: u32,
    /// The game hour of the block's first hour.
    pub block_first: i32,
    pub from: u16,
    pub to: u16,
    /// Each live row before `from`: slot, key, and per selected column its cell.
    pub start: Vec<(u32, u128, Vec<Held>)>,
    pub events: Vec<TrackEvent>,
    pub inserted: Vec<(u16, u32, u128)>,
    pub deleted: Vec<(u16, u32, u128)>,
    /// Each hour's status, `from` first.
    pub statuses: Vec<Status>,
    /// (hour, slot) of every row an hour read, for the hours read in part.
    pub read: Vec<(u16, u32)>,
}

/// What a stretch gathers from each part as it is fetched, at the block hours asked: the
/// selected columns' track events and the rows each hour read.
#[derive(Debug, Default)]
struct Tapped {
    events: Vec<TrackEvent>,
    read: Vec<(u16, u32)>,
}

impl Tapped {
    fn clear(&mut self) {
        self.events.clear();
        self.read.clear();
    }
}

/// One table of one run as it stood after an hour: its selected cells, strings and status.
pub struct TableReader {
    table: usize,
    kinds: Vec<Kind>,
    columns: Vec<usize>,
    run: u32,
    first_hour: i32,
    ordinals: u32,
    cells: TableState,
    strings: Strings,
    status: Status,
    read: bool,
    sampled: Option<i32>,
    /// A table read in part: the rows the hour read.
    view: Vec<u32>,
    /// The ordinal the cells hold; none before the first seek.
    ordinal: Option<u32>,
    block: u32,
    entry: TableEntry,
    next_part: usize,
    cursor: Cursor,
    changes: Changes,
    lives: Lives,
    marks: (Vec<(u16, u32)>, Vec<(u16, u32)>),
    /// Its cells keep a key index: a reader looking rows up by key.
    keyed: bool,
    /// A seek keeps the track events and read rows after its hour for the stretch read next
    /// ([`Self::keep_tracks`]): a part's tracks are read once, before the cursor consumes them.
    tracking: bool,
    pending: Tapped,
    restores: u32,
    /// A log table's cursor and each listed row's slot by ordinal.
    log: Option<Box<LogRead>>,
}

struct LogRead {
    cursor: LogCursor,
    /// The block hour the cursor started at: a part hands its hours from its first, and those
    /// before this one are already in the rows listed.
    from: u16,
    slots: BTreeMap<u64, u32>,
}

impl TableReader {
    /// A reader of `table` of `recording`'s run `run`, of its columns `columns` (every column
    /// the recording holds when none), not yet at any hour.
    pub fn new(recording: &Recording, run: u32, table: TableId, columns: Option<&[usize]>) -> io::Result<Self> {
        let held = recording.run(run)?;
        let index = table.index();
        if let Some(reason) = recording.withheld_reason(index) {
            return Err(Damage::wrap(DamageScope::Withheld { table: index as u32 }, invalid(reason.to_owned())));
        }
        let kinds = crate::columnar_frames::schema()?.swap_remove(index);
        let recorded = recording.columns()[index];
        let columns: Vec<usize> = match columns {
            Some(columns) => {
                let mut columns: Vec<usize> = columns.iter().copied().filter(|&column| column < recorded).collect();
                columns.sort_unstable();
                columns.dedup();
                columns
            }
            None => recording.read_columns(index),
        };
        let log = held.ordinals > 0 && unit::reader::class_of(&recording.table_pages(run, index, 0, 1)?, index)?;
        Ok(Self {
            table: index,
            log: log.then(|| Box::new(LogRead { cursor: LogCursor::new(&kinds, &columns, 0), from: 0, slots: BTreeMap::new() })),
            cursor: Cursor::new(&kinds, &columns, 0),
            cells: TableState::new(&kinds),
            changes: Changes { written: vec![Vec::new(); columns.len()], ..Changes::default() },
            kinds,
            columns,
            run,
            first_hour: held.first_hour,
            ordinals: held.ordinals,
            strings: Strings::default(),
            status: Status::NotRecorded,
            read: false,
            sampled: None,
            view: Vec::new(),
            ordinal: None,
            block: 0,
            entry: TableEntry::default(),
            next_part: 0,
            lives: Lives::default(),
            marks: (Vec::new(), Vec::new()),
            keyed: false,
            tracking: false,
            pending: Tapped::default(),
            restores: 0,
        })
    }

    /// Its cells keep a key index from now on, for [`Self::slot`] and [`TableView::slot`].
    pub fn index_keys(&mut self) {
        self.keyed = true;
        self.cells.index_keys();
    }

    /// Its seeks keep the track events of the rest of their block, for [`Self::read_stretch`].
    pub fn keep_tracks(&mut self) {
        self.tracking = true;
    }

    /// Whether the next step crosses into the next block after a block whose end packed the
    /// table: its slots are renumbered then.
    pub fn packs_on_step(&self) -> bool {
        self.log.is_none() && self.entry.packed && self.ordinal.is_some_and(|ordinal| (ordinal + 1) % BLOCK == 0)
    }

    /// The slot of the live row with `key`.
    pub fn slot(&self, key: u128) -> Option<u32> {
        self.cells.find(key)
    }

    /// Its cells, strings and column kinds, taken.
    pub fn into_parts(self) -> (TableState, Strings, Vec<Kind>) {
        (self.cells, self.strings, self.kinds)
    }

    pub fn table(&self) -> TableId {
        TableId::ALL[self.table]
    }

    pub fn run(&self) -> u32 {
        self.run
    }

    /// The selected columns, ascending.
    pub fn columns(&self) -> &[usize] {
        &self.columns
    }

    pub fn kinds(&self) -> &[Kind] {
        &self.kinds
    }

    /// The game hour its cells hold; none before the first seek.
    pub fn hour(&self) -> Option<i32> {
        self.ordinal.map(|ordinal| self.first_hour + ordinal as i32)
    }

    /// Its cells: the selected columns' (the others hold no value).
    pub fn cells(&self) -> &TableState {
        &self.cells
    }

    /// Its cells and strings as the store's readers read a table.
    pub fn view(&self) -> TableView<'_> {
        TableView::new(&self.cells, &self.kinds, &self.strings)
    }

    pub fn strings(&self) -> &Strings {
        &self.strings
    }

    pub fn status(&self) -> Status {
        self.status
    }

    /// Whether the hour read the table.
    pub fn was_read(&self) -> bool {
        self.read
    }

    pub fn sampled_at(&self) -> Option<i32> {
        self.sampled
    }

    /// For a table read in part, the rows the hour read, ascending; none otherwise.
    pub fn read_rows(&self) -> Option<&[u32]> {
        matches!(self.status, Status::Truncated { .. }).then_some(self.view.as_slice())
    }

    /// Whether the hour's status lets a reader count its rows.
    pub fn available(&self) -> bool {
        matches!(self.status, Status::Complete | Status::Truncated { .. })
    }

    /// What the hour it stepped to last changed.
    pub fn changes(&self) -> &Changes {
        &self.changes
    }

    /// The unavailable status's reason, as text.
    pub fn reason(&self) -> Option<&str> {
        match self.status {
            Status::Unavailable { reason } => Some(self.strings.get(reason).and_then(|text| std::str::from_utf8(text).ok()).unwrap_or("A table's reason is not text")),
            _ => None,
        }
    }

    /// How many times its cells were read from a restore.
    pub fn restores(&self) -> u32 {
        self.restores
    }

    /// The bytes it holds: its cells, strings and the cursor's open parts aside.
    pub fn held_bytes(&self) -> usize {
        self.cells.bytes().total()
    }

    fn damaged(&self, block: u32, error: io::Error) -> io::Error {
        Damage::wrap(DamageScope::Table { run: self.run, block, table: self.table as u32 }, error)
    }

    /// Its cells after the hour of run ordinal `ordinal`.
    pub fn seek(&mut self, recording: &Recording, ordinal: u32) -> io::Result<()> {
        if ordinal >= self.ordinals {
            return Err(invalid(format!("Run {} holds no ordinal {ordinal}", self.run)));
        }
        if self.log.is_some() {
            return self.seek_log(recording, ordinal);
        }
        let (block, hour) = (ordinal / BLOCK, (ordinal % BLOCK) as u16);
        let (restored, entry, first) = self.restore_before(recording, block)?;
        let source = recording.extents();
        let restoring = label("reader: restored cells");
        let (cells, meta) = unit::reader::restore(source, &entry, &self.kinds, &self.columns).map_err(|error| self.damaged(restored, error))?;
        self.cells = cells;
        self.restores += 1;
        if self.keyed {
            self.cells.index_keys();
        }
        self.strings = Strings::default();
        for (id, text) in meta.strings {
            self.strings.pin(id, &text)?;
        }
        drop(restoring);
        // From the run's first restore, block 0 goes on from its second hour.
        let (mut at, start) = if first { (restored, 1) } else { (restored + 1, 0) };
        while at < block {
            self.open_block(recording, at, start_of(at, first, restored, start))?;
            let last = self.block_hours(at) - 1;
            self.advance_to(recording, last, BLOCK as u16)?;
            if self.entry.packed {
                self.cells.pack()?;
                if self.keyed {
                    self.cells.index_keys();
                }
            }
            at += 1;
        }
        let begin = start_of(block, first, restored, start);
        self.open_block(recording, block, begin)?;
        if hour >= begin {
            if self.tracking {
                let mut pending = std::mem::take(&mut self.pending);
                let end = self.block_hours(block) - 1;
                self.advance_by(recording, hour, BLOCK as u16, Some((hour + 1, end, &mut pending)))?;
                self.pending = pending;
            } else {
                self.advance_to(recording, hour, BLOCK as u16)?;
            }
        }
        self.ordinal = Some(ordinal);
        self.settle(hour)?;
        self.changes.clear();
        self.changes.anew = true;
        self.changes.status_changed = true;
        Ok(())
    }

    /// Its cells after the next hour of the run; false at the run's end.
    pub fn step(&mut self, recording: &Recording) -> io::Result<bool> {
        let Some(ordinal) = self.ordinal else { return Err(invalid("A table reader steps before its first seek")) };
        if ordinal + 1 >= self.ordinals {
            return Ok(false);
        }
        let next = ordinal + 1;
        let (block, hour) = (next / BLOCK, (next % BLOCK) as u16);
        self.changes.clear();
        if self.log.is_some() {
            if block != self.block {
                self.open_block(recording, block, 0)?;
            }
            let before = (self.status, self.read, self.sampled);
            self.advance_log(recording, hour)?;
            self.ordinal = Some(next);
            self.settle(hour)?;
            self.changes.status_changed = before != (self.status, self.read, self.sampled);
            return Ok(true);
        }
        if block != self.block {
            if self.entry.packed {
                self.cells.pack()?;
                if self.keyed {
                    self.cells.index_keys();
                }
                self.changes.anew = true;
            }
            self.open_block(recording, block, 0)?;
        }
        let before = (self.status, self.read, self.sampled);
        self.advance_to(recording, hour, 1)?;
        for &(_, slot, key) in &self.lives.deleted {
            self.changes.deleted.push((slot, key));
        }
        for &(_, slot, _) in &self.lives.inserted {
            self.changes.inserted.push(slot);
        }
        self.changes.inserted.sort_unstable();
        self.changes.inserted.dedup();
        self.changes.inserted.retain(|&slot| self.cells.live().get(slot));
        self.ordinal = Some(next);
        self.settle(hour)?;
        self.changes.status_changed = before != (self.status, self.read, self.sampled);
        Ok(true)
    }

    /// Its cells after the hour of ordinal `ordinal`: advanced there from where they stand when it
    /// lies later in the run and no restore of the table lies between, else sought.
    pub fn go_to(&mut self, recording: &Recording, ordinal: u32) -> io::Result<()> {
        match self.ordinal {
            Some(at) if at == ordinal => {
                self.changes.clear();
                Ok(())
            }
            Some(at) if at < ordinal && ordinal < self.ordinals && self.log.is_some() => self.advance_log_to(recording, ordinal),
            Some(at) if at < ordinal && ordinal < self.ordinals && !self.restored_between(recording, at, ordinal / BLOCK)? => self.advance(recording, ordinal),
            _ => self.seek(recording, ordinal),
        }
    }

    /// Whether a restore of the table holds a later hour than ordinal `at` and lies before block
    /// `block`: a seek replays less from it than an advance from `at`.
    fn restored_between(&self, recording: &Recording, at: u32, block: u32) -> io::Result<bool> {
        for restored in (at / BLOCK..block).rev() {
            if recording.table_entry(self.run, restored, self.table)?.unit.restore.is_some() {
                return Ok((restored + 1) * BLOCK - 1 > at);
            }
        }
        Ok(false)
    }

    /// Its cells after the later hour of ordinal `ordinal`: the hours from those it holds replayed
    /// with one advance a block, each block end crossed as a step crosses it.
    fn advance(&mut self, recording: &Recording, ordinal: u32) -> io::Result<()> {
        let (block, hour) = (ordinal / BLOCK, (ordinal % BLOCK) as u16);
        while self.block < block {
            let last = self.block_hours(self.block) - 1;
            self.advance_to(recording, last, BLOCK as u16)?;
            if self.entry.packed {
                self.cells.pack()?;
                if self.keyed {
                    self.cells.index_keys();
                }
            }
            self.open_block(recording, self.block + 1, 0)?;
        }
        self.changes.clear();
        if self.tracking {
            let mut pending = std::mem::take(&mut self.pending);
            let end = self.block_hours(block) - 1;
            self.advance_by(recording, hour, BLOCK as u16, Some((hour + 1, end, &mut pending)))?;
            self.pending = pending;
        } else {
            self.advance_to(recording, hour, BLOCK as u16)?;
        }
        self.ordinal = Some(ordinal);
        self.settle(hour)?;
        self.changes.anew = true;
        self.changes.status_changed = true;
        Ok(())
    }

    /// The hours after the one it holds through ordinal `to`, which lies in the block of the next
    /// hour, read with one advance ([`Stretch`]); its cells then hold `to`.
    pub fn read_stretch(&mut self, recording: &Recording, to: u32) -> io::Result<Stretch> {
        if self.log.is_some() {
            return Err(invalid(format!("{:?} is a log: its rows never change, so it has no stretch of track events; read its rows at an hour", self.table())));
        }
        let Some(ordinal) = self.ordinal else { return Err(invalid("A table reader reads a stretch before its first seek")) };
        let next = ordinal + 1;
        if to < next || to >= self.ordinals || to / BLOCK != next / BLOCK {
            return Err(invalid(format!("A stretch to ordinal {to} after {ordinal} crosses a block or the run")));
        }
        let (block, from, last) = (next / BLOCK, (next % BLOCK) as u16, (to % BLOCK) as u16);
        if from == 0 {
            if self.entry.packed {
                self.cells.pack()?;
                if self.keyed {
                    self.cells.index_keys();
                }
            }
            self.open_block(recording, block, 0)?;
        }
        if !self.tracking {
            return Err(invalid("A stretch is read by a reader keeping tracks"));
        }
        let view = self.view();
        let start = view
            .rows()
            .filter_map(|slot| Some((slot, view.key(slot)?, self.columns.iter().map(|&column| Held::of(&view, column, slot)).collect())))
            .collect();
        // Parts the seek before fetched gave their events and marks then; open_block emptied them.
        let mut tapped = std::mem::take(&mut self.pending);
        tapped.events.retain(|event| from <= event.hour && event.hour <= last);
        tapped.read.retain(|&(hour, _)| from <= hour && hour <= last);
        self.changes.clear();
        self.advance_by(recording, last, BLOCK as u16, Some((from, last, &mut tapped)))?;
        let block_first = self.first_hour + (block * BLOCK) as i32;
        let mut statuses = Vec::with_capacity(usize::from(last - from) + 1);
        for hour in from..=last {
            statuses.push(unit::reader::status(&self.entry, hour, block_first).map_err(|error| self.damaged(block, error))?.0);
        }
        let Tapped { events, mut read } = tapped;
        if !statuses.iter().any(|status| matches!(status, Status::Truncated { .. })) {
            read.clear();
        }
        let stretch = Stretch {
            block,
            block_first,
            from,
            to: last,
            start,
            events,
            inserted: self.lives.inserted.clone(),
            deleted: self.lives.deleted.clone(),
            statuses,
            read,
        };
        self.ordinal = Some(to);
        self.settle(last)?;
        self.changes.anew = true;
        self.changes.status_changed = true;
        Ok(stretch)
    }

    /// The table's last restore at or before the end of block `block - 1` (the run's first
    /// restore for block 0): its block, entry, and whether it is the run's first.
    fn restore_before(&self, recording: &Recording, block: u32) -> io::Result<(u32, RestoreEntry, bool)> {
        let mut at = block;
        while at > 0 {
            at -= 1;
            let entry = recording.table_entry(self.run, at, self.table)?.unit;
            if let Some(restore) = entry.restore {
                return Ok((at, restore, false));
            }
            if at == 0 {
                let first = entry.first.ok_or_else(|| self.damaged(0, invalid("The run's first block holds no first restore")))?;
                return Ok((0, first, true));
            }
        }
        let entry = recording.table_entry(self.run, 0, self.table)?.unit;
        let first = entry.first.ok_or_else(|| self.damaged(0, invalid("The run's first block holds no first restore")))?;
        Ok((0, first, true))
    }

    fn block_hours(&self, block: u32) -> u16 {
        (self.ordinals - block * BLOCK).min(BLOCK) as u16
    }

    fn open_block(&mut self, recording: &Recording, block: u32, start: u16) -> io::Result<()> {
        self.entry = recording.table_entry(self.run, block, self.table)?.unit;
        self.block = block;
        self.next_part = 0;
        self.pending.clear();
        self.cursor = Cursor::new(&self.kinds, &self.columns, start);
        if let Some(log) = &mut self.log {
            log.cursor = LogCursor::new(&self.kinds, &self.columns, start);
            log.from = start;
        }
        Ok(())
    }

    /// The cursor brought to block hour `hour` in steps of `step` hours, parts fetched as it
    /// needs them; each step's written slots and lives gathered.
    fn advance_to(&mut self, recording: &Recording, hour: u16, step: u16) -> io::Result<()> {
        self.advance_by(recording, hour, step, None)
    }

    /// [`Self::advance_to`], each part fetched first giving its tracks' events and read rows at
    /// block hours `tapping.0..=tapping.1` to `tapping.2`, when given.
    fn advance_by(&mut self, recording: &Recording, hour: u16, step: u16, mut tapping: Option<(u16, u16, &mut Tapped)>) -> io::Result<()> {
        let hours = self.block_hours(self.block);
        let _cells = label("reader: cells");
        self.lives.deleted.clear();
        self.lives.inserted.clear();
        loop {
            let advanced = self.cursor.advance(hour, step, &mut self.cells, &mut self.changes.written, &mut self.lives).map_err(|error| self.damaged(self.block, error))?;
            match advanced {
                Advanced::Reached => return Ok(()),
                Advanced::Missing { .. } => {
                    // A table quiet for the rest of the block has no part for it.
                    let Some(part) = self.entry.parts.get(self.next_part) else { return Ok(()) };
                    let last = self.entry.parts.get(self.next_part + 1).map_or(hours - 1, |next| next.first - 1);
                    let _parts = label("reader: fetched parts");
                    let fetched = unit::reader::fetch(recording.extents(), part, last, &self.columns, self.cursor.prefix().as_ref()).map_err(|error| self.damaged(self.block, error))?;
                    for (id, text) in fetched.strings() {
                        self.strings.pin(*id, text)?;
                    }
                    if let Some((from, to, tapped)) = tapping.as_mut() {
                        tap(&fetched, &self.kinds, &self.columns, *from, *to, tapped)?;
                    }
                    self.cursor.push(fetched);
                    self.next_part += 1;
                }
            }
        }
    }

    /// Its status at block hour `hour`, and for a table read in part, the rows the hour read.
    fn settle(&mut self, hour: u16) -> io::Result<()> {
        let block_first = self.first_hour + (self.block * BLOCK) as i32;
        let (status, read, sampled, _) = unit::reader::status(&self.entry, hour, block_first).map_err(|error| self.damaged(self.block, error))?;
        self.status = status;
        self.read = read;
        self.sampled = sampled;
        self.view.clear();
        let truncated = matches!(status, Status::Truncated { .. });
        if self.log.is_some() {
            // A log keeps no read marks and an hour read in part forgets nothing: it lists its rows.
            if truncated {
                self.view.extend(self.cells.live().ones());
            }
            return Ok(());
        }
        if truncated {
            let (touched, read) = &mut self.marks;
            touched.clear();
            read.clear();
            self.cursor.marks(hour, hour, touched, read)?;
            let live = self.cells.live();
            self.view.extend(read.iter().map(|&(_, slot)| slot).filter(|&slot| live.get(slot)));
            self.view.extend(self.lives.inserted.iter().filter(|&&(at, _, _)| at == hour).map(|&(_, slot, _)| slot).filter(|&slot| live.get(slot)));
            self.view.sort_unstable();
            self.view.dedup();
        }
        Ok(())
    }
}

impl TableReader {
    /// A log's rows listed after the hour of run ordinal `ordinal`: those its restore before the
    /// hour's block lists, read by ordinal, then the block's hours through it.
    fn seek_log(&mut self, recording: &Recording, ordinal: u32) -> io::Result<()> {
        let (block, hour) = (ordinal / BLOCK, (ordinal % BLOCK) as u16);
        let (restored, entry, first) = self.restore_before(recording, block)?;
        let source = recording.extents();
        let restoring = label("reader: restored cells");
        let (state, meta) = unit::reader::restore(source, &entry, &self.kinds, &[]).map_err(|error| self.damaged(restored, error))?;
        self.restores += 1;
        let listed: Vec<u64> = state.listed().ok_or_else(|| self.damaged(restored, invalid("A log's restore holds no live set")))?.ones().collect();
        self.cells = TableState::new(&self.kinds);
        if self.keyed {
            self.cells.index_keys();
        }
        self.strings = Strings::default();
        for (id, text) in meta.strings {
            self.strings.pin(id, &text)?;
        }
        if !listed.is_empty() {
            let pages = recording.table_pages(self.run, self.table, 0, restored + 1)?;
            let log = self.log.as_deref_mut().expect("a log's reader");
            log.slots.clear();
            let (cells, strings, columns, slots) = (&mut self.cells, &mut self.strings, &self.columns, &mut log.slots);
            let mut failed = None;
            let read = unit::reader::log_rows_at(source, &pages, &self.kinds, self.table, columns, &listed, &mut |event| {
                let put = match event {
                    LogEvent::Named { id, text } => strings.pin(id, text),
                    LogEvent::Appended { ordinal, key, rows, at, .. } => cells.put_row_from(key, rows, at, columns).map(|slot| {
                        slots.insert(ordinal, slot);
                    }),
                    LogEvent::Forgotten { .. } => Ok(()),
                };
                if let Err(error) = put {
                    if failed.is_none() {
                        failed = Some(error);
                    }
                }
            });
            let held = log.slots.len();
            read.map_err(|error| self.damaged(restored, error))?;
            if let Some(error) = failed {
                return Err(error);
            }
            if held != listed.len() {
                return Err(self.damaged(restored, invalid("A log's listed rows are not all in its parts")));
            }
        } else if let Some(log) = self.log.as_deref_mut() {
            log.slots.clear();
        }
        drop(restoring);
        let (mut at, start) = if first { (restored, 1) } else { (restored + 1, 0) };
        while at < block {
            self.open_block(recording, at, start_of(at, first, restored, start))?;
            self.advance_log(recording, self.block_hours(at) - 1)?;
            at += 1;
        }
        let begin = start_of(block, first, restored, start);
        self.open_block(recording, block, begin)?;
        if hour >= begin {
            self.advance_log(recording, hour)?;
        }
        self.ordinal = Some(ordinal);
        self.settle(hour)?;
        self.changes.clear();
        self.changes.anew = true;
        self.changes.status_changed = true;
        Ok(())
    }

    /// A log's rows listed after the later hour of ordinal `ordinal`, its blocks' hours read
    /// from those it holds.
    fn advance_log_to(&mut self, recording: &Recording, ordinal: u32) -> io::Result<()> {
        let (block, hour) = (ordinal / BLOCK, (ordinal % BLOCK) as u16);
        self.changes.clear();
        while self.block < block {
            self.advance_log(recording, self.block_hours(self.block) - 1)?;
            self.open_block(recording, self.block + 1, 0)?;
        }
        self.advance_log(recording, hour)?;
        self.ordinal = Some(ordinal);
        self.settle(hour)?;
        self.changes.anew = true;
        self.changes.status_changed = true;
        Ok(())
    }

    /// The log's cursor brought to block hour `hour`, parts fetched as it needs them: each row
    /// appended put at the lowest free slot, each forgotten taken out. A row appended and
    /// forgotten since the changes were cleared is in neither.
    fn advance_log(&mut self, recording: &Recording, hour: u16) -> io::Result<()> {
        let hours = self.block_hours(self.block);
        let (run, block, table) = (self.run, self.block, self.table as u32);
        let damaged = |error| Damage::wrap(DamageScope::Table { run, block, table }, error);
        let _cells = label("reader: cells");
        let log = self.log.as_deref_mut().expect("a log's reader");
        loop {
            let (cells, columns, changes, slots, from) = (&mut self.cells, &self.columns, &mut self.changes, &mut log.slots, log.from);
            let mut failed = None;
            let advanced = log
                .cursor
                .advance(hour, &mut |event| {
                    if let Err(error) = listed(event, from, cells, columns, slots, changes) {
                        if failed.is_none() {
                            failed = Some(error);
                        }
                    }
                })
                .map_err(damaged)?;
            if let Some(error) = failed {
                return Err(error);
            }
            match advanced {
                Advanced::Reached => break,
                Advanced::Missing { .. } => {
                    // A log quiet for the rest of the block has no part for it.
                    let Some(part) = self.entry.parts.get(self.next_part) else { break };
                    let last = self.entry.parts.get(self.next_part + 1).map_or(hours - 1, |next| next.first - 1);
                    let _parts = label("reader: fetched parts");
                    let fetched = unit::reader::fetch(recording.extents(), part, last, &self.columns, log.cursor.prefix().as_ref()).map_err(damaged)?;
                    for (id, text) in fetched.strings() {
                        self.strings.pin(*id, text)?;
                    }
                    log.cursor.push(fetched);
                    self.next_part += 1;
                }
            }
        }
        self.changes.inserted.sort_unstable();
        Ok(())
    }
}

/// One event of a log's cursor at or after block hour `from` applied to the rows it lists.
fn listed(event: LogEvent<'_>, from: u16, cells: &mut TableState, columns: &[usize], slots: &mut BTreeMap<u64, u32>, changes: &mut Changes) -> io::Result<()> {
    match event {
        LogEvent::Appended { hour, .. } | LogEvent::Forgotten { hour, .. } if hour < from => {}
        LogEvent::Appended { ordinal, key, rows, at, .. } => {
            if slots.contains_key(&ordinal) {
                return Err(invalid(format!("A log appends row {ordinal}, which it lists already")));
            }
            let slot = cells.put_row_from(key, rows, at, columns)?;
            slots.insert(ordinal, slot);
            changes.inserted.push(slot);
        }
        LogEvent::Forgotten { from, to, .. } => {
            let gone: Vec<(u64, u32)> = slots.range(from..to).map(|(&ordinal, &slot)| (ordinal, slot)).collect();
            for (ordinal, slot) in gone {
                slots.remove(&ordinal);
                let key = cells.remove_row(slot)?;
                match changes.inserted.iter().position(|&held| held == slot) {
                    Some(at) => {
                        changes.inserted.swap_remove(at);
                    }
                    None => changes.deleted.push((slot, key)),
                }
            }
        }
        LogEvent::Named { .. } => {}
    }
    Ok(())
}

/// `part`'s track events of the selected columns and the rows its hours read, at block hours
/// `from..=to`.
fn tap(part: &Part, kinds: &[Kind], columns: &[usize], from: u16, to: u16, tapped: &mut Tapped) -> io::Result<()> {
    if part.last < from || part.first > to {
        return Ok(());
    }
    for (at, &column) in columns.iter().enumerate() {
        part.tracks(at, list_column(kinds[column]), |slot, born, held| {
            for &(hour, event) in held {
                if hour < from || hour > to {
                    continue;
                }
                let change = match event {
                    Event::Difference(change) => Change::Difference(change),
                    Event::Value(value) => Change::Value(value),
                    Event::Missing => Change::Missing,
                    Event::Unrecorded | Event::List(lists::GONE, _) => Change::Unrecorded,
                    Event::List(..) => Change::Written,
                };
                tapped.events.push(TrackEvent { column: at, slot, born, hour, change });
            }
        })?;
    }
    part.reads(from, to, &mut tapped.read)
}

/// Whether a column of `kind` is a list or opaque column, whose track events are list events.
pub fn list_column(kind: Kind) -> bool {
    matches!(kind, Kind::U32List | Kind::U64List | Kind::Opaque)
}

/// Whether a column of `kind` holds numbers a track's events change.
pub fn numeric(kind: Kind) -> bool {
    !matches!(kind, Kind::Bytes | Kind::U32List | Kind::U64List | Kind::Opaque)
}

/// The hour a block's cursor starts at: block 0 read on from the run's first restore starts at
/// its second hour.
fn start_of(block: u32, first: bool, restored: u32, start: u16) -> u16 {
    if first && block == restored { start } else { 0 }
}
