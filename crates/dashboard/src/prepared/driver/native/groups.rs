//! The country groups' native backend: each hour's operator reads the group tables the hour may
//! have changed from the walk's views, in the fields the groups' scan reads (only the rows the
//! hour's changes name, where the table it handed on before is complete), and hands
//! its ordered emit which tables moved and, of those, only what the scan reads of them otherwise
//! than the segment handed on before: the rows set and the keys gone, or a table whole where its
//! status or most of its rows moved. The emit applies them to its frame of the tables as the scan
//! reads them and observes it with the groups' scan and change publisher, as the hourly walk does,
//! a quiet step where nothing they read moved. It publishes the pages the walk publishes: the
//! scan, the changes and their catalogue.

use super::backend::{Native, Projected};
use crate::{
    Source,
    config::LoadContext,
    prepared::{driver::publication::Stage, frame_changes::FrameChanges},
    products::country_groups::{
        Scan,
        changes::Prepared as Changes,
        prepared::{self, TABLES},
    },
};
use dashboard_base::prepared::{
    roots::Published,
    series::{compress, decompress},
};
use dashboard_text::metadata::Extents;
use dashboard_world::{Hold, Needs, World};
use replay::{
    columnar_frames::{Segment, view_heap_bytes},
    columns,
    segments::{self, Operator, Sink, State},
};
use replay_data::rows::{
    CapitulationRow, CountryRow, FactionRow, RowCodec, TABLE_COUNT, TableId, TableRows,
    TableSchema, WarRelationRow, tables,
};
use replay_frame::frame::{ColumnView, DecodedFrame, Metrics, SharedView, TableView, not_selected};
use std::{
    io::{self, BufReader},
    mem::size_of,
    sync::atomic::AtomicBool,
};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// The tables the groups read, by position: [`Hour::moved`] holds a bit for each.
const READ: [TableId; 8] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::HumanControllers,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::Capitulation,
    TableId::LexicalTokens,
];

/// How the world holds each table the groups read, in [`READ`] order, and the fields a change to
/// which moves it ([`prepared::holds`]).
fn holds() -> io::Result<[(TableId, Hold); 8]> {
    let holds = prepared::holds()?;
    debug_assert!(holds.iter().map(|&(table, _)| table).eq(READ));
    Ok(holds)
}

/// Each row of a table as the scan reads it: the fields it reads, the others default.
fn country(row: &CountryRow) -> CountryRow {
    CountryRow {
        id: row.id,
        canonical_slot: row.canonical_slot,
        tag: row.tag.clone(),
        ..CountryRow::default()
    }
}
fn faction(row: &FactionRow) -> FactionRow {
    FactionRow {
        id: row.id,
        name: row.name.clone(),
        template_token: row.template_token,
        ..FactionRow::default()
    }
}
fn war(row: &WarRelationRow) -> WarRelationRow {
    WarRelationRow {
        first_country: row.first_country,
        second_country: row.second_country,
        start_hour: row.start_hour,
        end_hour: row.end_hour,
        first_was_instigator: row.first_was_instigator,
        ..WarRelationRow::default()
    }
}
fn capitulation(row: &CapitulationRow) -> CapitulationRow {
    CapitulationRow {
        country: row.country,
        capitulated: row.capitulated,
        capitulated_hour: row.capitulated_hour,
        ..CapitulationRow::default()
    }
}

/// How the scan reads a table's rows: each row as it reads it, and whether a row reads as a
/// row read before.
struct Reading<R> {
    read: fn(&R) -> R,
    same: fn(&R, &R) -> bool,
}

impl<R: RowCodec> Reading<R> {
    /// A table whose every field the scan reads.
    const WHOLE: Self = Self {
        read: R::clone,
        same: R::eq,
    };
}

const COUNTRIES: Reading<CountryRow> = Reading {
    read: country,
    same: |held, row| {
        held.id == row.id && held.canonical_slot == row.canonical_slot && held.tag == row.tag
    },
};
const FACTIONS: Reading<FactionRow> = Reading {
    read: faction,
    same: |held, row| {
        held.id == row.id && held.name == row.name && held.template_token == row.template_token
    },
};
const WARS: Reading<WarRelationRow> = Reading {
    read: war,
    same: |held, row| {
        held.first_country == row.first_country
            && held.second_country == row.second_country
            && held.start_hour == row.start_hour
            && held.end_hour == row.end_hour
            && held.first_was_instigator == row.first_was_instigator
    },
};
const CAPITULATIONS: Reading<CapitulationRow> = Reading {
    read: capitulation,
    same: |held, row| {
        held.country == row.country
            && held.capitulated == row.capitulated
            && held.capitulated_hour == row.capitulated_hour
    },
};

/// `view` with each of its rows as `read` reads them.
fn reduced<T: TableSchema>(view: &TableView, read: fn(&T::Row) -> T::Row) -> TableView {
    let rows =
        |rows: &TableRows| T::borrow(rows).map(|rows| T::wrap(rows.iter().map(read).collect()));
    match view {
        TableView::Complete(held) => rows(held).map_or_else(|| view.clone(), TableView::Complete),
        TableView::Truncated { rows: held, missing } => rows(held).map_or_else(
            || view.clone(),
            |rows| TableView::Truncated {
                rows,
                missing: *missing,
            },
        ),
        TableView::Unavailable { .. } | TableView::NotRecorded { .. } => view.clone(),
    }
}

fn ascending<R: RowCodec>(rows: &[R]) -> bool {
    rows.windows(2).all(|pair| pair[0].key() < pair[1].key())
}

/// `rows`, ascending by key, with the rows `set` and the keys `gone` applied, both ascending.
fn merge<R: RowCodec>(rows: &mut Vec<R>, set: Vec<R>, gone: &[u128]) {
    // Where no row goes, the rows held take their new values in place.
    let set = if gone.is_empty() {
        let mut missing = Vec::new();
        for row in set {
            match rows.binary_search_by_key(&row.key(), R::key) {
                Ok(position) => rows[position] = row,
                Err(_) => missing.push(row),
            }
        }
        missing
    } else {
        set
    };
    if set.is_empty() && gone.is_empty() {
        return;
    }
    let old = std::mem::take(rows);
    rows.reserve(old.len() + set.len());
    let mut set = set.into_iter().peekable();
    let mut gone = gone.iter().copied().peekable();
    for row in old {
        let key = row.key();
        while let Some(next) = set.next_if(|next| next.key() < key) {
            rows.push(next);
        }
        if let Some(next) = set.next_if(|next| next.key() == key) {
            rows.push(next);
            continue;
        }
        while gone.next_if(|&gone| gone < key).is_some() {}
        if gone.next_if(|&gone| gone == key).is_some() {
            continue;
        }
        rows.push(row);
    }
    rows.extend(set);
}

/// What an hour hands on of a table whose rows the scan reads moved.
#[derive(replay_spool::Spooled)]
enum Handed {
    /// The table as the scan reads it.
    Whole(SharedView),
    /// The rows set and the keys gone since the complete table handed on before, both
    /// ascending by key.
    Edits { set: TableRows, gone: Vec<u128> },
}

impl Handed {
    fn bytes(&self) -> usize {
        match self {
            Self::Whole(view) => view_heap_bytes(view),
            Self::Edits { set, gone } => set.heap_bytes() + gone.capacity() * size_of::<u128>(),
        }
    }
}

/// Whether `view` reads as `held`, a view as the scan read it.
fn reads_as<T: TableSchema>(
    held: &TableView,
    view: &TableView,
    reading: &Reading<T::Row>,
) -> bool {
    let same = |held: &TableRows, rows: &TableRows| match (T::borrow(held), T::borrow(rows)) {
        (Some(held), Some(rows)) => {
            held.len() == rows.len()
                && held.iter().zip(rows).all(|(held, row)| (reading.same)(held, row))
        }
        _ => false,
    };
    match (held, view) {
        (TableView::Complete(held), TableView::Complete(rows)) => same(held, rows),
        (
            TableView::Truncated {
                rows: held,
                missing: before,
            },
            TableView::Truncated { rows, missing },
        ) => before == missing && same(held, rows),
        (TableView::Unavailable { .. }, TableView::Unavailable { .. })
        | (TableView::NotRecorded { .. }, TableView::NotRecorded { .. }) => held == view,
        _ => false,
    }
}

/// What `view` hands on against `sent`, the table as the scan read it when handed on before,
/// which it becomes: none where the scan reads the same rows.
fn hand<T: TableSchema>(
    view: &TableView,
    sent: &mut Option<SharedView>,
    reading: &Reading<T::Row>,
) -> Option<Handed> {
    if let Some(held) = sent.as_mut()
        && let (TableView::Complete(old), TableView::Complete(new)) = (&**held, view)
        && let (Some(old), Some(new)) = (T::borrow(old), T::borrow(new))
        && ascending(old)
        && ascending(new)
    {
        let mut set = Vec::new();
        let mut gone = Vec::new();
        let (mut i, mut j) = (0, 0);
        loop {
            match (old.get(i), new.get(j)) {
                (Some(a), Some(b)) if a.key() == b.key() => {
                    if !(reading.same)(a, b) {
                        set.push((reading.read)(b));
                    }
                    i += 1;
                    j += 1;
                }
                (Some(a), Some(b)) if a.key() < b.key() => {
                    gone.push(a.key());
                    i += 1;
                }
                (Some(a), None) => {
                    gone.push(a.key());
                    i += 1;
                }
                (_, Some(b)) => {
                    set.push((reading.read)(b));
                    j += 1;
                }
                (None, None) => break,
            }
        }
        if set.is_empty() && gone.is_empty() {
            return None;
        }
        if set.len() + gone.len() < new.len() {
            // In place once the views handed on are the emit's alone.
            if let TableView::Complete(rows) = &mut **held
                && let Some(rows) = T::borrow_mut(rows)
            {
                merge(rows, set.clone(), &gone);
            }
            return Some(Handed::Edits {
                set: T::wrap(set),
                gone,
            });
        }
    }
    if sent
        .as_deref()
        .is_some_and(|held| reads_as::<T>(held, view, reading))
    {
        return None;
    }
    let new = SharedView::from(reduced::<T>(view, reading.read));
    *sent = Some(new.clone());
    Some(Handed::Whole(new))
}

/// What the hour's own changes to `table` hand on against `sent`, which holds it complete as the
/// scan read it the hour before ([`hand`] of the rows they name alone): Some(None) where the scan
/// reads the same rows, the edits where they are fewer than its rows; None where the changes
/// cannot tell, and the whole table does.
fn told<T: TableSchema>(
    world: &World,
    state: &State,
    (table, hold): (TableId, Hold),
    sent: &mut Option<SharedView>,
    reading: &Reading<T::Row>,
) -> io::Result<Option<Option<Handed>>> {
    let Some(held) = sent.as_mut() else {
        return Ok(None);
    };
    let (TableView::Complete(old), ColumnView::Complete(columns), Some(changes)) = (
        &**held,
        world.view(state, table),
        state.changes(table.index()),
    ) else {
        return Ok(None);
    };
    let Some(old) = T::borrow(old) else {
        return Ok(None);
    };
    if changes.truncated {
        return Ok(None);
    }
    let mask = match hold.of(table) {
        Hold::Whole => table.full_mask(),
        Hold::Fields(fields) => fields,
    };
    let at = |key: u128| old.binary_search_by_key(&key, |row| row.key());
    let mut slots: Vec<u32> = changes
        .changed
        .iter()
        .chain(&changes.inserted)
        .chain(&changes.fill_slots)
        .copied()
        .collect();
    slots.sort_unstable();
    slots.dedup();
    let mut set = Vec::new();
    for slot in slots {
        // A slot deleted since holds no row.
        let Some(key) = columns.table().key(slot) else {
            continue;
        };
        let row: T::Row = columns.row(slot, mask)?;
        if !at(key).is_ok_and(|at| (reading.same)(&old[at], &row)) {
            set.push((reading.read)(&row));
        }
    }
    let mut gone: Vec<u128> = changes
        .deleted_keys
        .iter()
        .copied()
        .filter(|&key| columns.find(key).is_none() && at(key).is_ok())
        .collect();
    if set.is_empty() && gone.is_empty() {
        return Ok(Some(None));
    }
    if set.len() + gone.len() >= columns.len() {
        return Ok(None);
    }
    set.sort_unstable_by_key(|row| row.key());
    gone.sort_unstable();
    gone.dedup();
    if let TableView::Complete(rows) = &mut **held
        && let Some(rows) = T::borrow_mut(rows)
    {
        merge(rows, set.clone(), &gone);
    }
    Ok(Some(Some(Handed::Edits {
        set: T::wrap(set),
        gone,
    })))
}

/// What the hour hands on of `table` against `sent`: from its own changes where they tell
/// ([`told`]), else from the table whole ([`hand`]).
fn hand_moved<T: TableSchema>(
    world: &World,
    state: &State,
    ask: (TableId, Hold),
    fresh: bool,
    sent: &mut Option<SharedView>,
    reading: &Reading<T::Row>,
) -> io::Result<Option<Handed>> {
    if !fresh && let Some(handed) = told::<T>(world, state, ask, sent, reading)? {
        return Ok(handed);
    }
    let view = world.copy(state, ask.0, ask.1)?;
    Ok(hand::<T>(&view, sent, reading))
}

/// What the hour hands on of `ask`'s table against `sent` ([`hand_moved`]); `fresh` where the
/// hour before is not the one `sent` was read at.
fn hand_table(
    world: &World,
    state: &State,
    ask: (TableId, Hold),
    fresh: bool,
    sent: &mut Option<SharedView>,
) -> io::Result<Option<Handed>> {
    match ask.0 {
        TableId::Countries => {
            hand_moved::<tables::Countries>(world, state, ask, fresh, sent, &COUNTRIES)
        }
        TableId::CountryLinks => {
            hand_moved::<tables::CountryLinks>(world, state, ask, fresh, sent, &Reading::WHOLE)
        }
        TableId::HumanControllers => {
            hand_moved::<tables::HumanControllers>(world, state, ask, fresh, sent, &Reading::WHOLE)
        }
        TableId::Factions => {
            hand_moved::<tables::Factions>(world, state, ask, fresh, sent, &FACTIONS)
        }
        TableId::FactionMembers => {
            hand_moved::<tables::FactionMembers>(world, state, ask, fresh, sent, &Reading::WHOLE)
        }
        TableId::WarRelations => {
            hand_moved::<tables::WarRelations>(world, state, ask, fresh, sent, &WARS)
        }
        TableId::Capitulation => {
            hand_moved::<tables::Capitulation>(world, state, ask, fresh, sent, &CAPITULATIONS)
        }
        TableId::LexicalTokens => {
            hand_moved::<tables::LexicalTokens>(world, state, ask, fresh, sent, &Reading::WHOLE)
        }
        _ => Ok(None),
    }
}

/// The complete view `held` with `set` and `gone` applied: a view of its own, since the scan
/// tells a table's rows apart by its view.
fn edited<T: TableSchema>(
    held: SharedView,
    set: TableRows,
    gone: &[u128],
) -> io::Result<SharedView> {
    let refused = || invalid("Country group edits name a table never handed on whole");
    let mut view = held.into_inner();
    let TableView::Complete(rows) = &mut view else {
        return Err(refused());
    };
    let rows = T::borrow_mut(rows).ok_or_else(refused)?;
    let set = T::take(set).map_err(|_| refused())?;
    merge(rows, set, gone);
    Ok(view.into())
}

/// `held` of `table` with `set` and `gone` applied ([`edited`]).
fn apply(
    table: TableId,
    held: SharedView,
    set: TableRows,
    gone: &[u128],
) -> io::Result<SharedView> {
    match table {
        TableId::Countries => edited::<tables::Countries>(held, set, gone),
        TableId::CountryLinks => edited::<tables::CountryLinks>(held, set, gone),
        TableId::HumanControllers => edited::<tables::HumanControllers>(held, set, gone),
        TableId::Factions => edited::<tables::Factions>(held, set, gone),
        TableId::FactionMembers => edited::<tables::FactionMembers>(held, set, gone),
        TableId::WarRelations => edited::<tables::WarRelations>(held, set, gone),
        TableId::Capitulation => edited::<tables::Capitulation>(held, set, gone),
        // The scan keeps the dictionary it read: the edited view is a copy.
        TableId::LexicalTokens => edited::<tables::LexicalTokens>(held, set, gone),
        _ => Err(invalid("Country group edits name a table the groups do not read")),
    }
}

/// One hour of a branch's group tables.
#[derive(replay_spool::Spooled)]
pub(crate) struct Hour {
    hour: i32,
    /// The hour is its segment's first: its tables are all handed on, and its scan begins a
    /// block there.
    starts: bool,
    /// The hour read the dictionary.
    lexical: bool,
    /// The tables the hour may have changed, a bit by position in [`READ`].
    moved: u8,
    /// Of those, what moved of the rows the scan reads.
    handed: Vec<(TableId, Handed)>,
}

impl Hour {
    /// The tables the hour may have changed.
    fn moved(&self) -> impl Iterator<Item = TableId> + '_ {
        READ.iter()
            .enumerate()
            .filter(|&(position, _)| self.moved & 1 << position != 0)
            .map(|(_, &table)| table)
    }
}

impl Projected for Hour {
    fn hour(&self) -> i32 {
        self.hour
    }
}

pub(crate) struct Projector {
    asks: [(TableId, Hold); 8],
    /// Each table as the scan read it when the segment last handed it on, by position in
    /// [`READ`].
    sent: [Option<SharedView>; 8],
}

impl Projector {
    fn new() -> io::Result<Self> {
        Ok(Self {
            asks: holds()?,
            sent: Default::default(),
        })
    }

    /// Nothing handed on: the next hour hands every table on.
    fn restart(&mut self) {
        self.sent = Default::default();
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.restart();
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("groups.columnar", Some("groups"));
        let world = hour.world;
        let opens = hour.starts || hour.opens;
        if opens {
            self.restart();
        }
        let fresh = opens || world.first();
        let mut moved = 0_u8;
        let mut handed = Vec::new();
        for (position, &(table, hold)) in self.asks.iter().enumerate() {
            let fields = match hold {
                Hold::Whole => u64::MAX,
                Hold::Fields(fields) => fields,
            };
            // The row decoder reads a table it cannot read whole anew every hour.
            if !(opens
                || world.touched(table, fields)
                || columns::status(hour.state, table) != columns::Status::Complete)
            {
                continue;
            }
            moved |= 1 << position;
            let sent = &mut self.sent[position];
            if let Some(change) = hand_table(world, hour.state, (table, hold), fresh, sent)? {
                engine_allocation::reserve(&mut handed, 1)?;
                handed.push((table, change));
            }
        }
        out.push(Hour {
            hour: hour.hour,
            starts: hour.starts,
            lexical: world.metrics().table_ns[TableId::LexicalTokens.index()] != 0,
            moved,
            handed,
        })
    }
}

/// A branch's groups, their change publisher, its first and last hours and its frame of the
/// group tables.
pub(crate) struct Branch {
    scan: Box<Scan>,
    changes: Box<Changes>,
    bounds: (i32, i32),
    frame: Option<DecodedFrame>,
}

impl Branch {
    fn new(scan: Scan, changes: Box<Changes>, bounds: (i32, i32)) -> Box<Self> {
        Box::new(Self {
            scan: Box::new(scan),
            changes,
            bounds,
            frame: None,
        })
    }

    fn observe(
        &mut self,
        source: &Source,
        context: &LoadContext,
        branch: u64,
        hour: Hour,
    ) -> io::Result<()> {
        let frame = self.frame.get_or_insert_with(|| DecodedFrame {
            branch,
            hour: hour.hour,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: String::new(),
                }
                .into()
            }),
        });
        if frame.branch != branch {
            return Err(invalid("Country groups crossed branches"));
        }
        if hour.starts {
            self.scan.begin_block();
        }
        frame.hour = hour.hour;
        let lexical = TableId::LexicalTokens.index();
        frame.metrics.table_ns[lexical] = u64::from(hour.lexical);
        let mut facts = FrameChanges {
            rows_changed: [false; TABLE_COUNT],
            table_read: [false; TABLE_COUNT],
            slot_read: [false; TABLE_COUNT],
            updates: None,
        };
        facts.table_read[lexical] = hour.lexical;
        for table in hour.moved() {
            facts.rows_changed[table.index()] = true;
        }
        for (table, handed) in hour.handed {
            let held = &mut frame.tables[table.index()];
            *held = match handed {
                Handed::Whole(view) => view,
                Handed::Edits { set, gone } => {
                    apply(table, std::mem::replace(held, not_selected()), set, &gone)?
                }
            };
        }
        self.changes
            .observe_changed(&mut self.scan, frame, &facts, source, context)
    }

    /// Publishes the changes kept since the last publication, and the scan's page.
    fn publish(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.finish_within(Some(self.bounds));
        self.changes.flush(writer, branch, &self.scan, cancelled)?;
        writer.write_at(&scan_section(branch), 0, self.scan.saved_bytes()?.as_slice())
    }

    /// What it goes on with after a publication: the change publisher's state.
    fn saved(&self) -> io::Result<Vec<u8>> {
        compress(&serde_json::to_vec(&*self.changes)?, 3)
    }

    /// The branch going on from its published scan page and [`Self::saved`].
    fn resumed(page: impl io::Read, saved: &[u8], bounds: (i32, i32)) -> io::Result<Box<Self>> {
        let scan = Scan::read(page)?;
        let changes = serde_json::from_slice(&decompress(saved)?)?;
        Ok(Self::new(scan, changes, bounds))
    }
}

/// The scan's published page of one branch.
fn scan_section(branch: u64) -> String {
    format!("country-groups-{branch}")
}

pub(crate) struct Groups<'p> {
    pub(crate) source: &'p Source,
    pub(crate) context: &'p LoadContext,
    pub(crate) extents: &'p Extents,
}

impl Groups<'_> {
    fn bounds(&self, branch: u64) -> io::Result<(i32, i32)> {
        self.extents
            .branch(branch)
            .map(|extent| extent.bounds)
            .ok_or_else(|| invalid("The recording holds no hour of the branch"))
    }
}

impl Native for Groups<'_> {
    type Operator = Projector;
    type Branch = Box<Branch>;

    fn tables(&self) -> &'static [TableId] {
        TABLES
    }

    fn needs(&self) -> io::Result<Needs> {
        Ok(Needs {
            frame: holds()?.to_vec(),
            ..Needs::default()
        })
    }

    fn operator(&self, _: &Segment) -> io::Result<Projector> {
        Projector::new()
    }

    fn branch(&self, branch: u64, _: i32) -> io::Result<Self::Branch> {
        let bounds = self.bounds(branch)?;
        Ok(Branch::new(
            Scan::within(Some(bounds)),
            Box::default(),
            bounds,
        ))
    }

    fn observe(
        &self,
        state: &mut Self::Branch,
        _: &mut Stage<'_, '_>,
        branch: u64,
        hour: Hour,
    ) -> io::Result<()> {
        state.observe(self.source, self.context, branch, hour)
    }

    fn flush(
        &self,
        state: &mut Self::Branch,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.publish(stage.pages()?, branch, cancelled)
    }

    // The scan goes on from its published page.
    fn save(&self, state: &mut Self::Branch) -> io::Result<Option<Vec<u8>>> {
        state.saved().map(Some)
    }

    fn resume(
        &self,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Self::Branch> {
        let page = stage
            .pages()?
            .read_at(&scan_section(branch), 0)?
            .ok_or_else(|| invalid("Its published page is missing"))?;
        Branch::resumed(BufReader::new(page), saved, self.bounds(branch)?)
    }
}

#[cfg(test)]
mod tests;
