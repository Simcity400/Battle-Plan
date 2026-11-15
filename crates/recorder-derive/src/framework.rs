//! What a derived-table writer is and what runs it: each hour, once the hour's recorded tables
//! are applied to the recorder's store, every writer whose inputs are complete is told the hour
//! and sets, deletes or unrecords its tables' rows. Its tables are lent to it for its hour
//! ([`Lent`]): each row goes into its table as it is set, a cell equal to the one held writing
//! nothing, and nothing of what it sets is held beside it; it reads its own tables through
//! [`Rows::table`]. A table it declares a log ([`Output::Log`]) is not lent: it appends rows to it
//! and forgets them by ordinal ([`Rows::append`], [`Rows::forget`]), landed through the store's log
//! path once its hour succeeds. A writer whose input is not complete is not run and its tables are
//! unavailable by name, their rows as they were; a writer that fails withholds its own tables for
//! the rest of the run and nothing else, the rows it set that hour undone. Runs are self-contained: every run
//! starts a writer fresh with empty tables. A writer is run only in an hour that moved one of its
//! inputs as it watches it ([`Read::watch`]), in its run's first hour, and after an hour it did not run, unless it declares an hourly duty
//! ([`Writer::hourly`]): an hour it is not run is one where it set nothing, its tables' statuses
//! as it last told them. A log is read by the rows each hour appended and forgot
//! ([`Watch::Appended`]); what a writer follows of a log across hours lives in its own tables,
//! which the store restores. A writer may also read [`definitions`], facts of the recorded content
//! no table holds, handed to the writers whenever they change (a session's facts packed, a
//! recording taken up). What a writer allocates in its own calls is charged to its memory label
//! (`writer: <name>`), what its tables add to the store to the store's.

use engine_allocation::labels::{self, Owner, Scope};
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    manifest::Status,
    state::{Cell, Kind},
    store::{
        Dropped, Given, Kept, Listed, Store, TableHour, TableView,
        lent::{Landing, Lent},
    },
};
use replay_data::{
    identity::Identity,
    rows::{Fields, LOGS, TableId, column},
};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    ops::Range,
    sync::OnceLock,
};

pub mod definitions;
#[cfg(test)]
mod tests;

pub use definitions::{Definition, Definitions, Facts, ProvinceRegions, Uses};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// Ids of the strings writers name start here, past every id a capture appends.
pub const DERIVED_STRINGS: u32 = 1 << 31;

/// What a writer is told of an input table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watch {
    /// The hours its watched columns change.
    Changed,
    /// Also the last cells of rows the game deleted, and each written cell's value before the hour.
    Dropped,
    /// Also the last cells of rows the game deleted, and nothing of a write: for a large table
    /// whose deleted rows alone a writer reads.
    Deleted,
    /// Every read, changed or not.
    EveryRead,
    /// A log's rows: the hours it appended or forgot rows ([`Hour::appended`],
    /// [`Hour::forgotten`]). The one watch of a log, and of a log alone. While such a log is not complete the writer's
    /// tables hold its status (the first of its logs not complete), reason and all, unless the
    /// writer tells a table's own.
    Appended,
}

/// Which of a table's columns a writer reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Columns {
    Every,
    These(&'static [usize]),
}

/// The columns of `R` named `names`, in order: a writer's [`Columns`] by field name.
pub const fn columns<R: Fields, const N: usize>(names: [&str; N]) -> [usize; N] {
    let mut out = [0; N];
    let mut at = 0;
    while at < N {
        out[at] = column::<R>(names[at]);
        at += 1;
    }
    out
}

impl Columns {
    fn holds(self, column: usize) -> bool {
        match self {
            Self::Every => true,
            Self::These(columns) => columns.contains(&column),
        }
    }
}

/// One table a writer writes, as it declares it ([`Writer::tables`]): state, lent to it for its
/// hour, or a log, which it appends rows to and forgets them from, its keys told once a run as
/// [`Keys`] says. Checked against the schema's [`LOGS`] and the store's logs at its run's first
/// hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Output {
    State(TableId),
    Log(TableId, Keys),
}

impl Output {
    pub const fn table(self) -> TableId {
        match self {
            Self::State(table) | Self::Log(table, _) => table,
        }
    }

    pub const fn log(self) -> bool {
        matches!(self, Self::Log(..))
    }

    /// A log whose every key of the run is held to refuse one given again ([`Keys::Run`]).
    const fn run_keyed(self) -> bool {
        matches!(self, Self::Log(_, Keys::Run))
    }
}

/// What a log's keys carry, which bounds what is held to refuse a key given again in the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    /// Nothing: every key the run gave is held, as ranges, and kept in the resume state.
    Run,
    /// The hour each is appended in, `hour as u32` at bit `.0`. A key of an earlier hour never
    /// equals one of this hour's, so only the hour's keys are held and none is kept past it; a key
    /// carrying another hour than the one written is the writer's failure, by name.
    Hour(u8),
}

impl Keys {
    /// The hour `key` carries, of [`Self::Hour`].
    const fn hour(self, key: u128) -> Option<i32> {
        match self {
            Self::Run => None,
            Self::Hour(at) => Some((key >> at) as u32 as i32),
        }
    }
}

/// One input of a writer.
#[derive(Clone, Copy, Debug)]
pub struct Read {
    pub table: TableId,
    /// The columns it reads.
    pub columns: Columns,
    pub watch: Watch,
    /// Its tables are unavailable while this input is not complete.
    pub required: bool,
}

/// A log's rows an hour hands a writer, in ordinal order: a view of the rows the store holds for
/// the hour, nothing copied and nothing read again from the file.
#[derive(Clone, Copy)]
pub struct LogRows<'a> {
    cells: Option<TableView<'a>>,
    /// The ordinal of the row at slot 0.
    from: u64,
    backlog: bool,
}

/// One row of [`LogRows`]: its cells at `slot` of [`LogRows::cells`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogRow {
    pub slot: u32,
    pub ordinal: u64,
    pub key: u128,
}

impl<'a> LogRows<'a> {
    const NONE: Self = Self { cells: None, from: 0, backlog: false };

    /// The list as it stood when the run began, each listed row handed once, not events of the
    /// hour: in a run's first hour.
    pub fn backlog(&self) -> bool {
        self.backlog
    }

    pub fn len(&self) -> usize {
        self.cells.map_or(0, |cells| cells.state().slots() as usize)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Each row, ascending by ordinal.
    pub fn rows(&self) -> impl Iterator<Item = LogRow> + 'a {
        let (cells, from) = (self.cells, self.from);
        cells.into_iter().flat_map(move |cells| {
            (0..cells.state().slots()).map(move |slot| LogRow { slot, ordinal: from + u64::from(slot), key: cells.key(slot).unwrap_or_default() })
        })
    }

    /// The rows' cells, by [`LogRow::slot`]; none when no row is handed.
    pub fn cells(&self) -> Option<TableView<'a>> {
        self.cells
    }
}

/// One hour as a writer sees it: every table of the store after the hour's recorded tables (and
/// the tables of the writers before it) were applied, and what the hour changed of each.
pub struct Hour<'a> {
    pub branch: u64,
    pub hour: i32,
    /// The run's first hour: a baseline, never events.
    pub first: bool,
    /// The writer did not run the hour before (the run's first, or its inputs were not
    /// complete): it reads its inputs whole.
    pub fresh: bool,
    /// The string ids of the writer's [`Writer::strings`], in order.
    pub strings: &'a [u32],
    store: &'a Store,
    /// The writer's own tables: it reads them through [`Rows::table`].
    own: &'a [Output],
    /// The first of them read here.
    misread: std::cell::Cell<Option<TableId>>,
    definitions: &'a Definitions,
}

impl<'a> Hour<'a> {
    /// The hour at `hour` of `store`, for a writer whose strings' ids are `strings`; no
    /// definition held ([`Self::defined`]).
    pub fn new(store: &'a Store, branch: u64, hour: i32, first: bool, fresh: bool, strings: &'a [u32]) -> Self {
        static UNPACKED: OnceLock<Definitions> = OnceLock::new();
        Self {
            branch,
            hour,
            first,
            fresh,
            strings,
            store,
            own: &[],
            misread: std::cell::Cell::new(None),
            definitions: UNPACKED.get_or_init(Definitions::unpacked),
        }
    }

    /// The hour with `definitions` held.
    pub fn defined(mut self, definitions: &'a Definitions) -> Self {
        self.definitions = definitions;
        self
    }

    /// The definitions the writers hold this hour, each kind or why it is absent.
    pub fn definitions(&self) -> &'a Definitions {
        self.definitions
    }

    /// The hour for a writer writing `own`, lent to it: a read of one here is the writer's
    /// failure, by name.
    fn owning(mut self, own: &'a [Output]) -> Self {
        self.own = own;
        self
    }

    /// `table` as the hour left it; of a log, read [`Self::appended`]. Not one of the writer's own
    /// tables, which it reads through [`Rows::table`].
    pub fn table(&self, table: TableId) -> TableView<'a> {
        if self.own.iter().any(|own| own.table() == table) && self.misread.get().is_none() {
            self.misread.set(Some(table));
        }
        self.store.view(table.index())
    }

    /// Log `table`'s rows the hour appended, ascending by ordinal, each its ordinal, key and
    /// cells; none of a state table. In a run's first hour they are the list as it stood when the
    /// run began, each listed row once ([`LogRows::backlog`]). A row the game changes in
    /// place comes as a forget of its old ordinal ([`Self::forgotten`]) and an append of the same
    /// key, often in one hour: a writer keyed by the row's key sees one update, one counting
    /// appends a new row. At a take-up's [`Writer::prime`] none are handed.
    pub fn appended(&self, table: TableId) -> LogRows<'a> {
        let view = self.table(table);
        let Some(from) = view.appended_from() else { return LogRows::NONE };
        LogRows { cells: Some(view), from, backlog: self.changes(table).whole }
    }

    /// The runs of ordinals log `table`'s game stopped listing this hour, rows appended this hour
    /// or before; the store keeps no cells of older rows, so a writer needing something of a row
    /// at its forget keeps it in its own tables from the row's append. An update in place forgets
    /// the old ordinal in the hour the same key is appended ([`Self::appended`]).
    pub fn forgotten(&self, table: TableId) -> &'a [std::ops::Range<u64>] {
        &self.changes(table).forgotten
    }

    /// Log `table`'s row count and the ordinals it lists as the hour began, before the writers'
    /// appends and forgets of it this hour; none of a state table. All the store keeps of a log:
    /// a writer finding a row's ordinal by its key after a resume keeps that ordinal in its own
    /// state tables, which the store restores.
    pub fn listed(&self, table: TableId) -> Option<(u64, &'a Listed)> {
        let held = self.store.table_state(table.index())?;
        Some((held.count()?, held.listed()?))
    }

    /// What the hour changed of `table`: nothing per row in a run's first hour
    /// ([`TableHour::whole`]), which a writer reads whole.
    pub fn changes(&self, table: TableId) -> &'a TableHour {
        &self.store.log().tables[table.index()]
    }

    pub fn status(&self, table: TableId) -> Status {
        self.table(table).status()
    }

    pub fn complete(&self, table: TableId) -> bool {
        self.status(table) == Status::Complete
    }

    /// Whether the hour inserted or deleted rows of `table`, or wrote one of `columns`.
    pub fn wrote(&self, table: TableId, columns: Columns) -> bool {
        let changes = self.changes(table);
        if changes.whole {
            return self.table(table).rows().next().is_some();
        }
        !changes.inserted.is_empty() || !changes.deleted.is_empty() || changes.written.iter().any(|(_, column)| columns.holds(usize::from(*column)))
    }

    /// Whether the hour inserted, deleted or wrote any row of `table`.
    pub fn changed(&self, table: TableId) -> bool {
        let changes = self.changes(table);
        if changes.whole {
            return self.table(table).rows().next().is_some();
        }
        !changes.inserted.is_empty() || !changes.deleted.is_empty() || !changes.written.is_empty()
    }

    /// The slots of `table`'s rows whose cells of `columns` the hour wrote, ascending, then the
    /// rows it inserted: in a run's first hour, every row.
    pub fn written_rows(&self, table: TableId, columns: Columns) -> Vec<u32> {
        let changes = self.changes(table);
        if changes.whole {
            return self.table(table).rows().collect();
        }
        let mut slots: Vec<u32> = changes.written.iter().filter(|(_, column)| columns.holds(usize::from(*column))).map(|&(slot, _)| slot).collect();
        slots.dedup();
        slots.extend_from_slice(&changes.inserted);
        slots
    }

    /// Each row of `table` the hour deleted, its key and its cells as the deletion found them: for
    /// an input read [`Watch::Dropped`] or [`Watch::Deleted`].
    pub fn dropped(&self, table: TableId) -> &'a Dropped {
        &self.changes(table).dropped
    }

    /// Each cell of an older row of `table` before the hour wrote it, (slot, column, value), a
    /// cell's first entry its value before the hour: for an input read [`Watch::Dropped`].
    pub fn previous(&self, table: TableId) -> &'a [(u32, u16, Kept)] {
        &self.changes(table).previous
    }

    /// The keys of `table`'s rows the hour deleted.
    pub fn deleted_keys(&self, table: TableId) -> impl Iterator<Item = u128> + 'a {
        self.changes(table).deleted.iter().map(|&(_, key)| key)
    }

    /// String `id`'s text.
    pub fn text(&self, id: u32) -> Option<&'a [u8]> {
        self.store.strings().get(id)
    }
}

/// One cell a writer sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    U8(u8),
    I8(i8),
    U16(u16),
    I16(i16),
    U32(u32),
    I32(i32),
    U64(u64),
    I64(i64),
    Bool(bool),
    OptionalI32(Option<i32>),
    OptionalU32(Option<u32>),
    OptionalI64(Option<i64>),
    OptionalU64(Option<u64>),
    /// A string id, of the capture's strings or made with [`Rows::text`].
    Bytes(u32),
    /// An opaque cell's bytes, made with [`Rows::opaque`].
    Opaque(u32),
    /// A list cell's values, made with [`Rows::list`].
    U32List(u32),
    /// Not known this hour.
    Unrecorded,
}

impl Value {
    fn kind(self) -> Option<Kind> {
        Some(match self {
            Self::U8(_) => Kind::U8,
            Self::I8(_) => Kind::I8,
            Self::U16(_) => Kind::U16,
            Self::I16(_) => Kind::I16,
            Self::U32(_) => Kind::U32,
            Self::I32(_) => Kind::I32,
            Self::U64(_) => Kind::U64,
            Self::I64(_) => Kind::I64,
            Self::Bool(_) => Kind::Bool,
            Self::OptionalI32(_) => Kind::OptionalI32,
            Self::OptionalU32(_) => Kind::OptionalU32,
            Self::OptionalI64(_) => Kind::OptionalI64,
            Self::OptionalU64(_) => Kind::OptionalU64,
            Self::Bytes(_) => Kind::Bytes,
            Self::Opaque(_) => Kind::Opaque,
            Self::U32List(_) => Kind::U32List,
            Self::Unrecorded => return None,
        })
    }

    /// The cell it sets, its opaque bytes among `opaques` and its list among `lists`; none when it
    /// unrecords.
    fn cell<'v>(self, opaques: &'v [Option<Vec<u8>>], lists: &'v [Option<Vec<u32>>]) -> Option<Cell<'v>> {
        Some(match self {
            Self::U8(value) => Cell::U8(value),
            Self::I8(value) => Cell::I8(value),
            Self::U16(value) => Cell::U16(value),
            Self::I16(value) => Cell::I16(value),
            Self::U32(value) => Cell::U32(value),
            Self::I32(value) => Cell::I32(value),
            Self::U64(value) => Cell::U64(value),
            Self::I64(value) => Cell::I64(value),
            Self::Bool(value) => Cell::Bool(value),
            Self::OptionalI32(value) => Cell::OptionalI32(value),
            Self::OptionalU32(value) => Cell::OptionalU32(value),
            Self::OptionalI64(value) => Cell::OptionalI64(value),
            Self::OptionalU64(value) => Cell::OptionalU64(value),
            Self::Bytes(value) => Cell::Bytes(value),
            Self::Opaque(index) => Cell::Opaque(opaques.get(index as usize).and_then(Option::as_deref).unwrap_or_default()),
            Self::U32List(index) => Cell::U32List(lists.get(index as usize).and_then(Option::as_deref).unwrap_or_default()),
            Self::Unrecorded => return None,
        })
    }
}

/// The strings writers name: by text, the next id, and those named since last held in the store.
struct Names {
    ids: HashMap<Box<[u8]>, u32>,
    next: u32,
    fresh: Vec<(u32, Box<[u8]>)>,
}

impl Default for Names {
    fn default() -> Self {
        Self { ids: HashMap::new(), next: DERIVED_STRINGS, fresh: Vec::new() }
    }
}

impl Names {
    /// The id of `text`, named now when it has none.
    fn id(&mut self, text: &[u8]) -> io::Result<u32> {
        if let Some(&id) = self.ids.get(text) {
            return Ok(id);
        }
        let id = self.next;
        self.next = id.checked_add(1).ok_or_else(|| invalid("Derived strings pass the last string id"))?;
        self.ids.insert(text.into(), id);
        self.fresh.push((id, text.into()));
        Ok(id)
    }

    /// Every string named since held in `store` for the run.
    fn pin(&mut self, store: &mut Store) -> io::Result<()> {
        for (id, text) in self.fresh.drain(..) {
            store.pin_string(id, &text)?;
        }
        Ok(())
    }

    /// Every string named since the last [`Self::pin`] unnamed.
    fn forget(&mut self) {
        for (id, text) in self.fresh.drain(..).rev() {
            self.ids.remove(&text);
            self.next = id;
        }
    }
}

/// Ordinals a writer forgot of a log this hour, as runs `start..end`.
#[derive(Default)]
struct Forgotten(BTreeMap<u64, u64>);

impl Forgotten {
    fn overlaps(&self, run: &Range<u64>) -> bool {
        self.0.range(..run.end).next_back().is_some_and(|(_, &end)| end > run.start)
    }

    /// `run`, none of it held.
    fn add(&mut self, mut run: Range<u64>) {
        if let Some((&start, &end)) = self.0.range(..run.start).next_back()
            && end == run.start
        {
            self.0.remove(&start);
            run.start = start;
        }
        if let Some(end) = self.0.remove(&run.end) {
            run.end = end;
        }
        self.0.insert(run.start, run.end);
    }

    fn runs(&self) -> impl Iterator<Item = Range<u64>> + '_ {
        self.0.iter().map(|(&start, &end)| start..end)
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A log a writer appends to and forgets from in its hour ([`Output::Log`]): what it did, held
/// until the hour succeeds and then landed through the store's log path ([`Self::land`]).
struct Appends {
    table: TableId,
    /// The log's row count as the hour began: the ordinal of the hour's first append.
    from: u64,
    /// What its keys carry, and the hour written.
    carried: Keys,
    hour: i32,
    keys: Vec<u128>,
    /// The appended rows' cells, each row's columns in turn; an opaque cell indexes `opaques`, a
    /// list cell `lists`.
    cells: Vec<Value>,
    opaques: Vec<Option<Vec<u8>>>,
    lists: Vec<Option<Vec<u32>>>,
    forgotten: Forgotten,
    /// The keys a key given again is refused against: the run's ([`Keys::Run`]) or the hour's
    /// ([`Keys::Hour`]), the hour's among them.
    given: Given,
}

impl Appends {
    fn new(table: TableId, from: u64, carried: Keys, hour: i32, given: Given) -> Self {
        Self { table, from, carried, hour, keys: Vec::new(), cells: Vec::new(), opaques: Vec::new(), lists: Vec::new(), forgotten: Forgotten::default(), given }
    }

    /// The hour appended to or forgot from the log.
    fn named(&self) -> bool {
        !self.keys.is_empty() || !self.forgotten.is_empty()
    }

    /// The hour's appends, in order, then its forgets, ascending, through the store's log path.
    fn land(&self, store: &mut Store) -> io::Result<()> {
        if !self.named() {
            return Ok(());
        }
        let index = self.table.index();
        let columns = store.kinds(index).len();
        let mut changes = store.table(index)?;
        let mut cells = Vec::with_capacity(columns);
        for (at, &key) in self.keys.iter().enumerate() {
            cells.clear();
            cells.extend(self.cells[at * columns..(at + 1) * columns].iter().map(|cell| cell.cell(&self.opaques, &self.lists)));
            let ordinal = changes.append(key, &cells)?;
            let told = self.from + at as u64;
            if ordinal != told {
                return Err(invalid(format!("{:?} appended row {ordinal} where its writer was told {told}: a table has one source", self.table)));
            }
        }
        for run in self.forgotten.runs() {
            changes.forget_run(run.start, run.end - run.start)?;
        }
        Ok(())
    }
}

/// What a writer sets of its tables this hour: each row written into its table as it is set, and
/// nothing of it held here; a log's appends and forgets held for the hour ([`Appends`]).
pub struct Rows<'a> {
    store: &'a Store,
    /// The writer's state tables, lent to it for its hour.
    lent: Vec<Lent>,
    /// The writer's logs.
    logs: Vec<Appends>,
    names: Names,
    /// Opaque bytes the writer made, each until a cell takes it.
    opaques: Vec<Option<Vec<u8>>>,
    /// Lists the writer made, each until a cell takes it.
    lists: Vec<Option<Vec<u32>>>,
    /// The store's memory label, which what a set adds to a table is charged to.
    held: Owner,
    statuses: Vec<(usize, Status)>,
    /// What the writer set that no table takes: the hour's error.
    fault: Option<io::Error>,
    /// Why the writer's hour is its failure: a row it set twice, a log written as state or a state
    /// table as a log, a log key given again or carrying another hour, a forget of a row not
    /// listed.
    refused: Option<String>,
    /// Lending no table, the keys set and deleted, for writers' tests.
    #[cfg(test)]
    logged: Option<Vec<(usize, u128, bool)>>,
    /// The keys of its lent tables it set to a row they did not hold and deleted, in a test noting
    /// them ([`note_keys`]).
    #[cfg(test)]
    noted: Option<Vec<(usize, u128, bool)>>,
}

#[cfg(test)]
thread_local! {
    static NOTING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// From now on, on this thread, every writer's hour notes the keys it sets to a row its table
/// did not hold, and those it deletes ([`Rows::keys`]), for writers' tests.
#[cfg(test)]
pub(crate) fn note_keys() {
    NOTING.set(true);
}

/// Refuses a row of `cells` that `table`, of `kinds`, cannot hold.
fn check(table: TableId, kinds: &[Kind], cells: &[Value], opaques: &[Option<Vec<u8>>], lists: &[Option<Vec<u32>>]) -> io::Result<()> {
    if cells.len() != kinds.len() {
        return Err(invalid(format!("A {table:?} row of {} cells", cells.len())));
    }
    for (cell, &kind) in cells.iter().zip(kinds) {
        if cell.kind().is_some_and(|held| held != kind) {
            return Err(invalid(format!("A {table:?} cell of another kind than its column")));
        }
        if let Value::Opaque(index) = cell
            && opaques.get(*index as usize).is_none_or(Option::is_none)
        {
            return Err(invalid(format!("A {table:?} cell names opaque bytes the writer did not make, or another cell took")));
        }
        if let Value::U32List(index) = cell
            && lists.get(*index as usize).is_none_or(Option::is_none)
        {
            return Err(invalid(format!("A {table:?} cell names a list the writer did not make, or another cell took")));
        }
    }
    Ok(())
}

impl<'a> Rows<'a> {
    fn new(store: &'a Store, lent: Vec<Lent>, logs: Vec<Appends>, names: Names, held: Owner) -> Self {
        Self {
            store,
            lent,
            logs,
            names,
            opaques: Vec::new(),
            lists: Vec::new(),
            held,
            statuses: Vec::new(),
            fault: None,
            refused: None,
            #[cfg(test)]
            logged: None,
            #[cfg(test)]
            noted: NOTING.get().then(Vec::new),
        }
    }

    /// Rows lending no table, logging the keys set and deleted ([`Self::keys`]), for writers'
    /// tests.
    #[cfg(test)]
    pub(crate) fn logging(store: &'a Store) -> Self {
        Self { logged: Some(Vec::new()), ..Self::new(store, Vec::new(), Vec::new(), Names::default(), Owner::RUN) }
    }

    /// Rows writing into `lent`, tables `store` lent, for writers' tests.
    #[cfg(test)]
    pub(crate) fn lending(store: &'a Store, lent: Vec<Lent>) -> Self {
        Self::new(store, lent, Vec::new(), Names::default(), Owner::RUN)
    }

    /// Where in [`Self::lent`] `table` is; none, the hour's error, when the writer does not write it.
    fn lent_at(&mut self, table: TableId, key: u128, set: bool) -> Option<usize> {
        let index = table.index();
        if let Some(at) = self.lent.iter().position(|lent| lent.table() == index) {
            return Some(at);
        }
        if self.logs.iter().any(|log| log.table == table) {
            let what = if set { "set" } else { "deleted" };
            self.refused.get_or_insert_with(|| format!("it {what} a row of log table {table:?}: a log's rows are appended and forgotten"));
            return None;
        }
        #[cfg(test)]
        if let Some(logged) = &mut self.logged {
            logged.push((index, key, set));
            return None;
        }
        let _ = (key, set);
        self.fault.get_or_insert_with(|| invalid(format!("A writer names {table:?}, which it does not write")));
        None
    }

    /// A Bytes cell naming `text`, among the strings the run's writers name.
    pub fn text(&mut self, text: &[u8]) -> Value {
        match self.names.id(text) {
            Ok(id) => Value::Bytes(id),
            Err(error) => {
                self.fault.get_or_insert(error);
                Value::Unrecorded
            }
        }
    }

    /// An Opaque cell holding `bytes`.
    pub fn opaque(&mut self, bytes: Vec<u8>) -> Value {
        self.opaques.push(Some(bytes));
        Value::Opaque((self.opaques.len() - 1) as u32)
    }

    /// A U32List cell holding `values`.
    pub fn list(&mut self, values: Vec<u32>) -> Value {
        self.lists.push(Some(values));
        Value::U32List((self.lists.len() - 1) as u32)
    }

    /// `table`'s row `key` holds `cells`, one per column: written into the table now. A key set
    /// twice in one hour fails the writer's hour by name.
    pub fn set(&mut self, table: TableId, key: u128, cells: Vec<Value>) {
        if self.fault.is_some() || self.refused.is_some() {
            return;
        }
        let Some(at) = self.lent_at(table, key, true) else { return };
        let lent = &mut self.lent[at];
        if let Err(error) = check(table, lent.kinds(), &cells, &self.opaques, &self.lists) {
            self.fault = Some(error);
            return;
        }
        let (opaques, lists) = (&self.opaques, &self.lists);
        #[cfg(test)]
        let before = self.noted.is_some().then(|| (lent.view(self.store.strings()).slot(key).is_some(), lent.changed()));
        let set = {
            let _label = labels::enter(self.held, labels::Kind::Operator);
            lent.set(key, cells.iter().map(|cell| cell.cell(opaques, lists)))
        };
        #[cfg(test)]
        if let (Some(noted), Some((held, changed))) = (&mut self.noted, before)
            && (!held || lent.changed() > changed)
        {
            noted.push((table.index(), key, true));
        }
        match set {
            Ok(true) => {}
            Ok(false) => self.refused = Some(format!("it set one {} row twice in one hour", table.words())),
            Err(error) => self.fault = Some(error),
        }
        for cell in &cells {
            match cell {
                Value::Opaque(index) => self.opaques[*index as usize] = None,
                Value::U32List(index) => self.lists[*index as usize] = None,
                _ => {}
            }
        }
    }

    /// `table` holds no row `key` once the writer's hour ends, unless the hour sets it.
    pub fn delete(&mut self, table: TableId, key: u128) {
        if let Some(at) = self.lent_at(table, key, false) {
            #[cfg(test)]
            if let Some(noted) = &mut self.noted {
                noted.push((table.index(), key, false));
            }
            let _label = labels::enter(self.held, labels::Kind::Operator);
            self.lent[at].delete(key);
        }
    }

    /// Where in [`Self::logs`] `table` is; none when the hour has failed, or when `table` is not
    /// one of the writer's logs: a state table of its own fails its hour by name.
    fn log_at(&mut self, table: TableId, what: &str) -> Option<usize> {
        if self.fault.is_some() || self.refused.is_some() {
            return None;
        }
        if let Some(at) = self.logs.iter().position(|log| log.table == table) {
            return Some(at);
        }
        if self.lent.iter().any(|lent| lent.table() == table.index()) {
            self.refused = Some(format!("it {what} state table {table:?}: a state table's rows are set and deleted"));
        } else {
            self.fault = Some(invalid(format!("A writer names {table:?}, which it does not write")));
        }
        None
    }

    /// Appends row `key` to `table`, one of the writer's logs, its cells one per column as
    /// [`Self::set`] takes them: the row's ordinal, the rows the log appended before it. It lands
    /// through the store's log path once the writer's hour succeeds. None once the hour has failed;
    /// an append to a state table, of a key the log was given before in its run, or to a log keyed
    /// by its hour ([`Keys::Hour`]) of a key carrying another hour, fails the writer's hour by name.
    pub fn append(&mut self, table: TableId, key: u128, cells: Vec<Value>) -> Option<u64> {
        let at = self.log_at(table, "appended to")?;
        if let Err(error) = check(table, self.store.kinds(table.index()), &cells, &self.opaques, &self.lists) {
            self.fault = Some(error);
            return None;
        }
        let log = &mut self.logs[at];
        if let Some(carried) = log.carried.hour(key)
            && carried != log.hour
        {
            self.refused = Some(format!("it gave log table {table:?} key {key:#x} of hour {carried} in hour {}: its keys carry the hour they are appended in", log.hour));
            return None;
        }
        if log.given.holds(key) {
            self.refused = Some(format!("it gave log table {table:?} key {key:#x} again: a log holds a key once a run"));
            return None;
        }
        let Some(ordinal) = log.from.checked_add(log.keys.len() as u64) else {
            self.fault = Some(invalid(format!("Log table {table:?} passes 2^64 rows")));
            return None;
        };
        let start = log.cells.len();
        for cell in cells {
            let cell = match cell {
                Value::Opaque(index) => match self.opaques[index as usize].take() {
                    Some(bytes) => {
                        log.opaques.push(Some(bytes));
                        Value::Opaque((log.opaques.len() - 1) as u32)
                    }
                    None => {
                        log.cells.truncate(start);
                        self.fault = Some(invalid(format!("A {table:?} cell names opaque bytes another cell took")));
                        return None;
                    }
                },
                Value::U32List(index) => match self.lists[index as usize].take() {
                    Some(values) => {
                        log.lists.push(Some(values));
                        Value::U32List((log.lists.len() - 1) as u32)
                    }
                    None => {
                        log.cells.truncate(start);
                        self.fault = Some(invalid(format!("A {table:?} cell names a list another cell took")));
                        return None;
                    }
                },
                cell => cell,
            };
            log.cells.push(cell);
        }
        log.given.add(key);
        log.keys.push(key);
        Some(ordinal)
    }

    /// `table`, one of the writer's logs, no longer lists row `ordinal`, appended this hour or
    /// before.
    pub fn forget(&mut self, table: TableId, ordinal: u64) {
        self.forget_run(table, ordinal, 1);
    }

    /// [`Self::forget`] of rows `first..first + count`. A forget from a state table, or of a row
    /// the log never appended or no longer lists, fails the writer's hour by name.
    pub fn forget_run(&mut self, table: TableId, first: u64, count: u64) {
        let Some(at) = self.log_at(table, "forgot rows of") else { return };
        if count == 0 {
            return;
        }
        let listed = self.store.table_state(table.index()).and_then(|held| held.listed());
        let log = &mut self.logs[at];
        let next = log.from + log.keys.len() as u64;
        let Some(end) = first.checked_add(count).filter(|&end| end <= next) else {
            self.refused = Some(format!("it forgot {table:?} row {}, which the log never appended", first.max(next)));
            return;
        };
        let older_listed = listed.is_some_and(|listed| listed.all(first, end.min(log.from)));
        if !older_listed || log.forgotten.overlaps(&(first..end)) {
            let gone = |ordinal: u64| (ordinal < log.from && !listed.is_some_and(|listed| listed.get(ordinal))) || log.forgotten.overlaps(&(ordinal..ordinal + 1));
            let ordinal = (first..end).find(|&ordinal| gone(ordinal)).unwrap_or(first);
            self.refused = Some(format!("it forgot {table:?} row {ordinal}, which the log forgot before"));
            return;
        }
        log.forgotten.add(first..end);
    }

    /// `table` as the writer's hour sees it: one of its own as written so far, its older rows as
    /// the hour began and its new rows as set; any other as [`Hour::table`].
    pub fn table(&self, table: TableId) -> TableView<'_> {
        match self.lent.iter().find(|lent| lent.table() == table.index()) {
            Some(lent) => lent.view(self.store.strings()),
            None => self.store.view(table.index()),
        }
    }

    /// The keys set and deleted of `table`, for writers' tests.
    #[cfg(test)]
    pub(crate) fn keys(&self, table: TableId) -> (Vec<u128>, Vec<u128>) {
        let (mut sets, mut deletes) = (Vec::new(), Vec::new());
        for &(_, key, set) in self.logged.iter().chain(&self.noted).flatten().filter(|(index, ..)| *index == table.index()) {
            if set { sets.push(key) } else { deletes.push(key) }
        }
        sets.sort_unstable();
        deletes.sort_unstable();
        (sets, deletes)
    }

    /// `table`'s status this hour, when not complete.
    pub fn status(&mut self, table: TableId, status: Status) {
        let index = table.index();
        match self.statuses.iter_mut().find(|(held, _)| *held == index) {
            Some((_, held)) => *held = status,
            None => self.statuses.push((index, status)),
        }
    }

    /// `table` is unavailable this hour for `reason`, a text the writer made up.
    pub fn unavailable(&mut self, table: TableId, reason: &[u8]) {
        if let Value::Bytes(reason) = self.text(reason) {
            self.status(table, Status::Unavailable { reason });
        }
    }
}

/// A derived-table writer.
pub trait Writer: Send {
    fn name(&self) -> &'static str;
    /// Of its rules and of its resume blob.
    fn version(&self) -> u32;
    /// The tables it writes, each state or a log.
    fn tables(&self) -> &'static [Output];
    fn reads(&self) -> &'static [Read];
    /// It runs every hour, whatever its inputs did: what it sets depends on the hour itself (rows
    /// living one hour, a date falling due, an hour that brought it nothing).
    fn hourly(&self) -> bool {
        false
    }
    /// Texts it writes in Bytes cells beside the capture's own strings.
    fn strings(&self) -> &'static [&'static str] {
        &[]
    }
    /// Definitions it reads ([`definitions`]): whenever the definitions held change, its next
    /// hour is fresh.
    fn definitions(&self) -> &'static [Uses] {
        &[]
    }
    /// A run starts: its tables are empty.
    fn start(&mut self);
    /// Its hour: it sets, deletes and unrecords its state tables' rows through `out`, which writes
    /// each into its table as it comes, and reads them through [`Rows::table`]; it appends to and
    /// forgets from its logs through `out` by ordinal.
    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()>;
    fn save(&self, blob: &mut Bytes) -> io::Result<()>;
    fn load(&mut self, blob: &[u8]) -> io::Result<()>;
    /// A run taken up at a commit, after [`Self::load`]: what it holds across hours, read from
    /// the store as it stands at `hour`, the commit's last, emitting nothing; it compares from
    /// the next hour as a writer never stopped does.
    fn prime(&mut self, hour: &Hour<'_>) {
        let _ = hour;
    }
}

/// Every registered writer of a run, in dependency order, and what runs them.
pub struct Writers {
    writers: Vec<Box<dyn Writer>>,
    /// Each writer's withholding reason for the rest of the run.
    withheld: Vec<Option<String>>,
    /// Each writer ran the hour before.
    ran: Vec<bool>,
    /// The strings writers named.
    names: Names,
    /// Each writer's time so far: its hour and its tables' changes.
    spent: Vec<u64>,
    /// The writers' tables and reads were checked against the logs since the run began.
    checked: bool,
    definitions: Definitions,
    reader: definitions::Reader,
    /// Each writer's memory label.
    owners: Vec<Owner>,
    /// The store's memory label.
    held: Owner,
    /// By writer, the keys each of its logs keyed by the run ([`Keys::Run`]) was given in the run,
    /// in the order it declares them; a log keyed by its hour holds its keys for the hour alone.
    given: Vec<Vec<Given>>,
}

/// The memory label of `writer`'s own work, named once.
fn owner(writer: &dyn Writer) -> Owner {
    if labels::ENABLED { labels::owner(&format!("writer: {}", writer.name())) } else { Owner::RUN }
}

/// A writer's run-keyed logs' keys before it appends.
fn fresh_given(writer: &dyn Writer) -> Vec<Given> {
    vec![Given::default(); writer.tables().iter().filter(|output| output.run_keyed()).count()]
}

fn class(log: bool) -> &'static str {
    if log { "a log" } else { "state" }
}

impl Writers {
    pub fn new(writers: Vec<Box<dyn Writer>>) -> Self {
        let withheld = vec![None; writers.len()];
        let ran = vec![false; writers.len()];
        let spent = vec![0; writers.len()];
        let owners = writers.iter().map(|writer| owner(&**writer)).collect();
        let given = writers.iter().map(|writer| fresh_given(&**writer)).collect();
        Self {
            writers,
            withheld,
            ran,
            names: Names::default(),
            spent,
            checked: false,
            definitions: Definitions::unpacked(),
            reader: definitions::read,
            owners,
            held: labels::owner("store"),
            given,
        }
    }

    /// Charges what the thread allocates to writer `at`'s label until the scope drops.
    fn label(&self, at: usize) -> Scope {
        labels::enter(self.owners[at], labels::Kind::Operator)
    }

    /// Charges what the thread allocates to the store's label, the recorder's own: the derived
    /// tables' rows and strings.
    fn store_label(&self) -> Scope {
        labels::enter(self.held, labels::Kind::Operator)
    }

    /// Every kind of definition a writer declares.
    pub fn wanted(&self) -> Vec<Definition> {
        let mut wanted: Vec<Definition> = Vec::new();
        for uses in self.writers.iter().flat_map(|writer| writer.definitions()) {
            if !wanted.contains(&uses.definition) {
                wanted.push(uses.definition);
            }
        }
        wanted
    }

    /// The definitions held, of a session or a recording; they outlive runs.
    pub fn definitions(&self) -> &Definitions {
        &self.definitions
    }

    /// Holds `definitions`: when they differ from those held, each writer declaring any reads its
    /// inputs whole at its next hour. Hours before keep what was written.
    pub fn define(&mut self, definitions: Definitions) {
        if definitions == self.definitions {
            return;
        }
        self.definitions = definitions;
        for (at, writer) in self.writers.iter().enumerate() {
            if !writer.definitions().is_empty() {
                self.ran[at] = false;
            }
        }
    }

    /// Holds the definitions the writers declare, read from game facts pack `facts` of a
    /// recording of `identity`; nothing is read when none declares any.
    pub fn define_from(&mut self, facts: &Facts, identity: &Identity) {
        let wanted = self.wanted();
        if !wanted.is_empty() {
            let read = (self.reader)(facts, identity, &wanted);
            self.define(read);
        }
    }

    /// Reads definitions from packs with `reader` instead of the game facts' readers.
    pub fn read_definitions_with(&mut self, reader: definitions::Reader) {
        self.reader = reader;
    }

    /// Refuses by name a table a writer declares a log ([`Output`]) that the schema's [`LOGS`] or
    /// the store holds as state, or the other way round; a log whose keys carry their hour past
    /// the bits a key holds; a read of a log with another watch than [`Watch::Appended`], and that
    /// watch on a state table. `logs` are the store's, by table.
    pub fn check(&self, logs: &[bool]) -> io::Result<()> {
        for writer in &self.writers {
            for output in writer.tables() {
                let (table, log) = (output.table(), output.log());
                if let Output::Log(_, Keys::Hour(at)) = *output
                    && at > 96
                {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} declares the keys of {table:?} carry their hour at bit {at}: a key's 128 bits end before the hour's 32", writer.name())));
                }
                let (schema, held) = (LOGS.contains(&table), logs.get(table.index()).copied().unwrap_or(false));
                let holder = if log != schema {
                    ("the schema", schema)
                } else if log != held {
                    ("the store", held)
                } else {
                    continue;
                };
                return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} declares {table:?} {}, which {} holds as {}", writer.name(), class(log), holder.0, class(holder.1))));
            }
            for read in writer.reads() {
                let log = logs.get(read.table.index()).copied().unwrap_or(false);
                let refused = if log && read.watch != Watch::Appended {
                    format!("{} reads log table {:?} with watch {:?}: a log is read by its appended rows", writer.name(), read.table, read.watch)
                } else if !log && read.watch == Watch::Appended {
                    format!("{} reads state table {:?} with watch {:?}: only a log is read by its appended rows", writer.name(), read.table, read.watch)
                } else {
                    continue;
                };
                return Err(io::Error::new(io::ErrorKind::InvalidInput, refused));
            }
        }
        Ok(())
    }

    /// Each writer's name and time so far.
    pub fn spent(&self) -> Vec<(&'static str, u64)> {
        self.writers.iter().map(|writer| writer.name()).zip(self.spent.iter().copied()).collect()
    }

    /// Every table the writers write, ascending.
    pub fn tables(&self) -> Vec<usize> {
        let mut tables: Vec<usize> = self
            .writers
            .iter()
            .flat_map(|writer| writer.tables().iter().map(|output| output.table().index()))
            .collect();
        tables.sort_unstable();
        tables
    }

    /// The tables written from `table`: each writer's that reads it or a table written from it,
    /// ascending.
    #[cfg(any(test, feature = "test-support"))]
    pub fn derived_from(&self, table: TableId) -> Vec<usize> {
        let mut from = vec![table.index()];
        for writer in &self.writers {
            if writer.reads().iter().any(|read| from.contains(&read.table.index())) {
                from.extend(writer.tables().iter().map(|written| written.table().index()));
            }
        }
        from.remove(0);
        from.sort_unstable();
        from.dedup();
        from
    }

    /// Adds `writer` after the others, before a run starts.
    pub fn push(&mut self, writer: Box<dyn Writer>) {
        self.owners.push(owner(&*writer));
        self.given.push(fresh_given(&*writer));
        self.writers.push(writer);
        self.withheld.push(None);
        self.ran.push(false);
        self.spent.push(0);
        self.checked = false;
    }

    /// Every table the writers read or write, which they look up by key.
    pub fn keyed(&self) -> Vec<usize> {
        let mut tables: Vec<usize> = self
            .writers
            .iter()
            .flat_map(|writer| writer.tables().iter().map(|output| output.table().index()).chain(writer.reads().iter().map(|read| read.table.index())))
            .collect();
        tables.sort_unstable();
        tables.dedup();
        tables
    }

    /// Every table a writer reads as `watch`.
    fn watched(&self, watch: Watch) -> Vec<usize> {
        self.writers.iter().flat_map(|writer| writer.reads().iter().filter(|read| read.watch == watch).map(|read| read.table.index())).collect()
    }

    /// Makes `store`'s hour log keep what the writers read of replaced cells ([`Watch::Dropped`])
    /// and of deleted rows ([`Watch::Deleted`]).
    fn keep(&self, store: &mut Store) {
        for table in self.watched(Watch::Dropped) {
            store.keep_previous(table);
        }
        for table in self.watched(Watch::Deleted) {
            store.keep_dropped(table);
        }
    }

    /// A run starts: every writer fresh, none withheld, the strings they name named afresh in the
    /// run's store.
    pub fn start_run(&mut self) {
        for at in 0..self.writers.len() {
            let _label = self.label(at);
            self.writers[at].start();
            self.given[at] = fresh_given(&*self.writers[at]);
        }
        self.withheld.iter_mut().for_each(|held| *held = None);
        self.ran.iter_mut().for_each(|ran| *ran = false);
        self.names = Names::default();
        self.checked = false;
    }

    /// The string id of `text`, named by a writer; held in `store`.
    fn string(&mut self, store: &mut Store, text: &[u8]) -> io::Result<u32> {
        let _label = self.store_label();
        let id = self.names.id(text)?;
        self.names.pin(store)?;
        Ok(id)
    }

    /// Runs every writer on the hour `store` holds, each one's tables lent to it while it runs and
    /// landed before the next runs.
    pub fn hour(&mut self, branch: u64, hour: i32, first: bool, store: &mut Store) -> io::Result<()> {
        if !self.checked {
            self.check(store.logs())?;
            self.checked = true;
        }
        {
            let _label = self.store_label();
            for table in self.keyed() {
                store.index(table);
            }
            self.keep(store);
        }
        for at in 0..self.writers.len() {
            let clock = std::time::Instant::now();
            let (name, tables, reads) = {
                let writer = &self.writers[at];
                (writer.name(), writer.tables(), writer.reads())
            };
            if let Some(table) = tables.iter().map(|output| output.table()).find(|table| landed(store, table.index())) {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{table:?} is {name}'s, and the hour landed it before {name} ran: a table has one source")));
            }
            let mut blocked = self.withheld[at].clone();
            if blocked.is_none() {
                for read in reads.iter().filter(|read| read.required) {
                    let status = store.view(read.table.index()).status();
                    if status != Status::Complete {
                        blocked = Some(input_missing(store, read.table, status));
                        break;
                    }
                }
            }
            if blocked.is_none() {
                for uses in self.writers[at].definitions().iter().filter(|uses| uses.required) {
                    if let Some(reason) = self.definitions.reason(&uses.definition) {
                        blocked = Some(format!("the game's definition {} is not held: {reason}", uses.definition));
                        break;
                    }
                }
            }
            let mut statuses = Vec::new();
            let mut landings = Vec::new();
            let mut appended = Vec::new();
            let idle = self.ran[at] && !first && !due(&*self.writers[at], store, hour);
            if blocked.is_none() && !idle {
                let declared = self.writers[at].strings();
                let mut strings = Vec::with_capacity(declared.len());
                for text in declared {
                    strings.push(self.string(store, text.as_bytes())?);
                }
                let mut lent = Vec::with_capacity(tables.len());
                let mut logs = Vec::new();
                let mut lending = Ok(());
                let mut given = self.given[at].iter_mut();
                for output in tables {
                    let index = output.table().index();
                    match *output {
                        Output::State(_) => match store.lend(index) {
                            Ok(table) => lent.push(table),
                            Err(error) => lending = Err(error),
                        },
                        Output::Log(table, carried) => match store.table_state(index).and_then(|held| held.count()) {
                            Some(from) => {
                                let keys = match carried {
                                    Keys::Run => given.next().map(std::mem::take).unwrap_or_default(),
                                    Keys::Hour(_) => Given::default(),
                                };
                                logs.push(Appends::new(table, from, carried, hour, keys));
                            }
                            None => lending = Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{table:?} is {name}'s log, and the store holds it as state"))),
                        },
                    }
                    if lending.is_err() {
                        break;
                    }
                }
                if let Err(error) = lending {
                    for table in lent {
                        store.take_back(table);
                    }
                    return Err(error);
                }
                let (result, misread, rows) = {
                    let mut rows = Rows::new(store, lent, logs, std::mem::take(&mut self.names), self.held);
                    let seen = Hour::new(store, branch, hour, first, !self.ran[at], &strings).owning(tables).defined(&self.definitions);
                    let result = {
                        let _label = labels::enter(self.owners[at], labels::Kind::Operator);
                        self.writers[at].hour(&seen, &mut rows)
                    };
                    let Rows { lent, logs, names, statuses, fault, refused, .. } = rows;
                    (result, seen.misread.get(), (lent, logs, names, statuses, fault, refused))
                };
                let (lent, mut logs, names, set_statuses, fault, refused) = rows;
                self.names = names;
                for (held, log) in self.given[at].iter_mut().zip(logs.iter_mut().filter(|log| log.carried == Keys::Run)) {
                    *held = std::mem::take(&mut log.given);
                }
                let failure = match result {
                    Err(error) if error.kind() != io::ErrorKind::InvalidData => Some(Err(error)),
                    Err(error) => Some(Ok(error.to_string())),
                    Ok(()) => match (fault, refused, misread) {
                        (Some(fault), _, _) => Some(Err(fault)),
                        (None, Some(refused), _) => Some(Ok(refused)),
                        (None, None, Some(table)) => Some(Ok(format!("it read its own {} as an input", table.words()))),
                        (None, None, None) => None,
                    },
                };
                match failure {
                    None => {
                        let _label = labels::enter(self.held, labels::Kind::Operator);
                        landings = lent.into_iter().map(|table| store.take_back(table)).collect();
                        appended = logs;
                        self.names.pin(store)?;
                        statuses = set_statuses;
                    }
                    Some(failure) => {
                        let _label = labels::enter(self.held, labels::Kind::Operator);
                        let mut undone = Ok(());
                        for mut table in lent {
                            undone = undone.and(table.undo());
                            store.take_back(table);
                        }
                        self.names.forget();
                        undone?;
                        let reason = format!("the recorder stopped working this out: {}", failure?);
                        self.withheld[at] = Some(reason.clone());
                        blocked = Some(reason);
                    }
                }
            }
            self.ran[at] = blocked.is_none();
            let inherited = reads
                .iter()
                .filter(|read| read.watch == Watch::Appended)
                .map(|read| store.view(read.table.index()).status())
                .find(|status| *status != Status::Complete);
            let (mut landings, mut appended) = (landings.into_iter(), appended.into_iter());
            for output in tables {
                let index = output.table().index();
                let (landing, log): (Option<Landing>, Option<Appends>) = match output {
                    Output::State(_) => (landings.next(), None),
                    Output::Log(..) => (None, appended.next()),
                };
                let held = store.view(index);
                let (held_status, held_sampled) = (held.status(), held.sampled_at());
                let status = match &blocked {
                    Some(reason) => {
                        let reason = match held_status {
                            Status::Unavailable { reason: held_id } if held.string(held_id) == Some(reason.as_bytes()) => held_id,
                            _ => self.string(store, reason.as_bytes())?,
                        };
                        Status::Unavailable { reason }
                    }
                    None if idle => held_status,
                    None => statuses.iter().find(|(held, _)| *held == index).map(|(_, status)| *status).or(inherited).unwrap_or(Status::Complete),
                };
                let read = blocked.is_none();
                let sampled = if read { Some(hour) } else { held_sampled };
                let sectioned = read && (landing.as_ref().is_some_and(Landing::named) || log.as_ref().is_some_and(Appends::named));
                let _label = self.store_label();
                store.status(index, status, read, sampled, sectioned)?;
                if read {
                    if let Some(landing) = landing {
                        store.land(landing)?;
                    }
                    if let Some(log) = log {
                        log.land(store)?;
                    }
                }
            }
            {
                let _label = self.store_label();
                store.settle();
            }
            self.spent[at] += clock.elapsed().as_nanos() as u64;
        }
        Ok(())
    }

    /// Every writer's resume state: the strings writers named, then by writer its name, version,
    /// withholding, blob, and the keys each of its run-keyed logs was given; none of a log keyed by
    /// its hour.
    pub fn save(&self) -> io::Result<Vec<u8>> {
        let mut out = Bytes::new();
        out.varint(u64::from(self.names.next))?;
        let mut strings: Vec<(&Box<[u8]>, &u32)> = self.names.ids.iter().collect();
        strings.sort_unstable_by_key(|(_, id)| **id);
        out.varint(strings.len() as u64)?;
        for (text, id) in strings {
            out.varint(u64::from(*id))?;
            out.varint(text.len() as u64)?;
            out.raw(text)?;
        }
        out.varint(self.writers.len() as u64)?;
        for (((writer, withheld), ran), given) in self.writers.iter().zip(&self.withheld).zip(&self.ran).zip(&self.given) {
            let name = writer.name().as_bytes();
            out.varint(name.len() as u64)?;
            out.raw(name)?;
            out.varint(u64::from(writer.version()))?;
            out.u8(u8::from(*ran))?;
            match withheld {
                None => out.u8(0)?,
                Some(reason) => {
                    out.u8(1)?;
                    out.varint(reason.len() as u64)?;
                    out.raw(reason.as_bytes())?;
                }
            }
            let mut blob = Bytes::new();
            writer.save(&mut blob)?;
            out.varint(blob.bytes().len() as u64)?;
            out.raw(blob.bytes())?;
            out.varint(given.len() as u64)?;
            for keys in given {
                out.varint(keys.len() as u64)?;
                for (start, end) in keys.ranges() {
                    out.raw(&start.to_le_bytes())?;
                    out.raw(&end.to_le_bytes())?;
                }
            }
        }
        Ok(out.into_bytes())
    }

    /// Each writer's name and the bytes of its resume state as [`Self::save`] writes it.
    pub fn blob_sizes(&self) -> io::Result<Vec<(&'static str, usize)>> {
        self.writers
            .iter()
            .map(|writer| {
                let mut blob = Bytes::new();
                writer.save(&mut blob)?;
                Ok((writer.name(), blob.bytes().len()))
            })
            .collect()
    }

    /// Takes up a run after [`Self::save`] wrote `bytes`, the strings writers named held in
    /// `store`: a writer saved by another version, or not saved, withholds its tables for the rest
    /// of the run.
    pub fn load(&mut self, bytes: &[u8], store: &mut Store) -> io::Result<()> {
        let held = self.store_label();
        self.keep(store);
        let mut reader = Reader::new(bytes);
        self.names = Names { next: reader.varint_u32()?, ..Names::default() };
        let count = reader.count()?;
        if count > bytes.len() {
            return Err(invalid("A writers' resume state counts more strings than it holds"));
        }
        for _ in 0..count {
            let id = reader.varint_u32()?;
            let length = reader.count()?;
            let text = reader.raw(length)?;
            store.pin_string(id, text)?;
            self.names.ids.insert(text.into(), id);
        }
        drop(held);
        let count = reader.count()?;
        let mut saved = HashMap::new();
        for _ in 0..count {
            let length = reader.count()?;
            let name = String::from_utf8(reader.raw(length)?.to_vec()).map_err(|_| invalid("A writer's name is not UTF-8"))?;
            let version = reader.varint_u32()?;
            let ran = reader.u8()? != 0;
            let withheld = match reader.u8()? {
                0 => None,
                _ => {
                    let length = reader.count()?;
                    Some(String::from_utf8_lossy(reader.raw(length)?).into_owned())
                }
            };
            let length = reader.count()?;
            let blob = reader.raw(length)?;
            let logs = reader.count()?;
            if logs > bytes.len() {
                return Err(invalid("A writer's resume state counts more logs than it holds"));
            }
            let mut given = Vec::with_capacity(logs);
            for _ in 0..logs {
                let ranges = reader.count()?;
                if ranges > bytes.len() {
                    return Err(invalid("A writer's resume state counts more key ranges than it holds"));
                }
                let mut keys = Given::default();
                for _ in 0..ranges {
                    let mut wide = || -> io::Result<u128> { Ok(u128::from_le_bytes(reader.raw(16)?.try_into().map_err(|_| invalid("A key range ends early"))?)) };
                    let start = wide()?;
                    keys.hold_range(start, wide()?);
                }
                given.push(keys);
            }
            saved.insert(name, (version, ran, withheld, blob, given));
        }
        reader.finish()?;
        for (at, writer) in self.writers.iter_mut().enumerate() {
            let _label = labels::enter(self.owners[at], labels::Kind::Operator);
            writer.start();
            let fresh = fresh_given(&**writer);
            (self.withheld[at], self.ran[at], self.given[at]) = match saved.remove(writer.name()) {
                Some((version, ran, withheld, blob, given)) if version == writer.version() && given.len() == fresh.len() => {
                    writer.load(blob)?;
                    (withheld, ran, given)
                }
                _ => (Some(OTHER_VERSION.to_owned()), false, fresh),
            };
        }
        Ok(())
    }
}

impl Writers {
    /// A run taken up at a commit whose last hour is `hour`, after [`Self::load`]: each writer
    /// that ran that hour primes itself from `store`, the tables as that hour left them.
    pub fn prime(&mut self, store: &Store, branch: u64, hour: i32) {
        for (at, writer) in self.writers.iter_mut().enumerate() {
            if self.withheld[at].is_none() && self.ran[at] {
                let _label = labels::enter(self.owners[at], labels::Kind::Operator);
                writer.prime(&Hour::new(store, branch, hour, false, false, &[]).defined(&self.definitions));
            }
        }
    }
}

/// Whether `writer` has a reason to run this hour: it declares an hourly duty, or the hour moved
/// one of its inputs as it watches it ([`moved`]).
fn due(writer: &dyn Writer, store: &Store, hour: i32) -> bool {
    if writer.hourly() {
        return true;
    }
    writer.reads().iter().any(|read| moved(store, read, hour))
}

/// Whether the hour moved `read`'s table as `read` watches it: its status or log changed, or, read
/// every read, its sample hour as the store holds it is `hour` (changed or not; a section without
/// that stamp is no read), or else its watched columns or rows changed.
fn moved(store: &Store, read: &Read, hour: i32) -> bool {
    let changes = &store.log().tables[read.table.index()];
    changes.status_changed
        || changes.appended > 0
        || !changes.forgotten.is_empty()
        || match read.watch {
            Watch::Appended => false,
            Watch::EveryRead => store.view(read.table.index()).sampled_at() == Some(hour),
            Watch::Changed | Watch::Dropped | Watch::Deleted => {
                changes.whole || !changes.inserted.is_empty() || !changes.deleted.is_empty() || changes.written.iter().any(|(_, column)| read.columns.holds(usize::from(*column)))
            }
        }
}

/// Whether the hour, before its writers, changed `table` or told its status: a run's first hour
/// tells every landed table's.
fn landed(store: &Store, table: usize) -> bool {
    let changes = &store.log().tables[table];
    changes.status_changed
        || changes.present
        || changes.appended > 0
        || !changes.forgotten.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty()
        || !changes.written.is_empty()
}

/// Why a writer's tables are withheld after a take-up of another version's saved work.
const OTHER_VERSION: &str = "another version of the recorder worked this out, and this version cannot take up its saved work";

/// Why a writer's tables are withheld for an input at `status`, in the words a user reads: what
/// the input holds and why it is missing, with the input's own reason.
fn input_missing(store: &Store, input: TableId, status: Status) -> String {
    let what = input.words();
    match status {
        Status::NotRecorded => format!("{what}: not recorded"),
        Status::Complete => format!("{what}: complete"),
        Status::Truncated { missing } => format!("{what}: read only in part ({missing} rows not read)"),
        Status::Unavailable { reason } => match store.strings().get(reason) {
            Some(text) if !text.is_empty() => format!("{what}: not available ({})", String::from_utf8_lossy(text)),
            _ => format!("{what}: not available"),
        },
    }
}
