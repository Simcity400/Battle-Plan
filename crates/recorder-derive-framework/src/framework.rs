
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

pub const DERIVED_STRINGS: u32 = 1 << 31;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Watch {
    Changed,
    Dropped,
    Deleted,
    EveryRead,
    Appended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Columns {
    Every,
    These(&'static [usize]),
}

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

    const fn run_keyed(self) -> bool {
        matches!(self, Self::Log(_, Keys::Run))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    Run,
    Hour(u8),
}

impl Keys {
    const fn hour(self, key: u128) -> Option<i32> {
        match self {
            Self::Run => None,
            Self::Hour(at) => Some((key >> at) as u32 as i32),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Read {
    pub table: TableId,
    pub columns: Columns,
    pub watch: Watch,
    pub required: bool,
}

#[derive(Clone, Copy)]
pub struct LogRows<'a> {
    cells: Option<TableView<'a>>,
    from: u64,
    backlog: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogRow {
    pub slot: u32,
    pub ordinal: u64,
    pub key: u128,
}

impl<'a> LogRows<'a> {
    const NONE: Self = Self { cells: None, from: 0, backlog: false };

    pub fn backlog(&self) -> bool {
        self.backlog
    }

    pub fn len(&self) -> usize {
        self.cells.map_or(0, |cells| cells.state().slots() as usize)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn rows(&self) -> impl Iterator<Item = LogRow> + 'a {
        let (cells, from) = (self.cells, self.from);
        cells.into_iter().flat_map(move |cells| {
            (0..cells.state().slots()).map(move |slot| LogRow { slot, ordinal: from + u64::from(slot), key: cells.key(slot).unwrap_or_default() })
        })
    }

    pub fn cells(&self) -> Option<TableView<'a>> {
        self.cells
    }
}

pub struct Hour<'a> {
    pub branch: u64,
    pub hour: i32,
    pub first: bool,
    pub fresh: bool,
    pub format: u16,
    pub strings: &'a [u32],
    store: &'a Store,
    own: &'a [Output],
    misread: std::cell::Cell<Option<TableId>>,
    definitions: &'a Definitions,
}

impl<'a> Hour<'a> {
    pub fn new(store: &'a Store, branch: u64, hour: i32, first: bool, fresh: bool, strings: &'a [u32]) -> Self {
        static UNPACKED: OnceLock<Definitions> = OnceLock::new();
        Self {
            branch,
            hour,
            first,
            fresh,
            format: replay_data::FORMAT_VERSION,
            strings,
            store,
            own: &[],
            misread: std::cell::Cell::new(None),
            definitions: UNPACKED.get_or_init(Definitions::unpacked),
        }
    }

    pub fn with_format(mut self, format: u16) -> Self {
        self.format = format;
        self
    }

    pub fn defined(mut self, definitions: &'a Definitions) -> Self {
        self.definitions = definitions;
        self
    }

    pub fn definitions(&self) -> &'a Definitions {
        self.definitions
    }

    fn owning(mut self, own: &'a [Output]) -> Self {
        self.own = own;
        self
    }

    pub fn table(&self, table: TableId) -> TableView<'a> {
        if self.own.iter().any(|own| own.table() == table) && self.misread.get().is_none() {
            self.misread.set(Some(table));
        }
        self.store.view(table.index())
    }

    pub fn appended(&self, table: TableId) -> LogRows<'a> {
        let view = self.table(table);
        let Some(from) = view.appended_from() else { return LogRows::NONE };
        LogRows { cells: Some(view), from, backlog: self.changes(table).whole }
    }

    pub fn forgotten(&self, table: TableId) -> &'a [std::ops::Range<u64>] {
        &self.changes(table).forgotten
    }

    pub fn listed(&self, table: TableId) -> Option<(u64, &'a Listed)> {
        let held = self.store.table_state(table.index())?;
        Some((held.count()?, held.listed()?))
    }

    pub fn changes(&self, table: TableId) -> &'a TableHour {
        &self.store.log().tables[table.index()]
    }

    pub fn status(&self, table: TableId) -> Status {
        self.table(table).status()
    }

    pub fn complete(&self, table: TableId) -> bool {
        self.status(table) == Status::Complete
    }

    pub fn wrote(&self, table: TableId, columns: Columns) -> bool {
        let changes = self.changes(table);
        if changes.whole {
            return self.table(table).rows().next().is_some();
        }
        !changes.inserted.is_empty() || !changes.deleted.is_empty() || changes.written.iter().any(|(_, column)| columns.holds(usize::from(*column)))
    }

    pub fn changed(&self, table: TableId) -> bool {
        let changes = self.changes(table);
        if changes.whole {
            return self.table(table).rows().next().is_some();
        }
        !changes.inserted.is_empty() || !changes.deleted.is_empty() || !changes.written.is_empty()
    }

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

    pub fn dropped(&self, table: TableId) -> &'a Dropped {
        &self.changes(table).dropped
    }

    pub fn previous(&self, table: TableId) -> &'a [(u32, u16, Kept)] {
        &self.changes(table).previous
    }

    pub fn deleted_keys(&self, table: TableId) -> impl Iterator<Item = u128> + 'a {
        self.changes(table).deleted.iter().map(|&(_, key)| key)
    }

    pub fn text(&self, id: u32) -> Option<&'a [u8]> {
        self.store.strings().get(id)
    }
}

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
    Bytes(u32),
    Opaque(u32),
    U32List(u32),
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

    fn pin(&mut self, store: &mut Store) -> io::Result<()> {
        for (id, text) in self.fresh.drain(..) {
            store.pin_string(id, &text)?;
        }
        Ok(())
    }

    fn forget(&mut self) {
        for (id, text) in self.fresh.drain(..).rev() {
            self.ids.remove(&text);
            self.next = id;
        }
    }
}

#[derive(Default)]
struct Forgotten(BTreeMap<u64, u64>);

impl Forgotten {
    fn overlaps(&self, run: &Range<u64>) -> bool {
        self.0.range(..run.end).next_back().is_some_and(|(_, &end)| end > run.start)
    }

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

struct Appends {
    table: TableId,
    from: u64,
    carried: Keys,
    hour: i32,
    keys: Vec<u128>,
    cells: Vec<Value>,
    opaques: Vec<Option<Vec<u8>>>,
    lists: Vec<Option<Vec<u32>>>,
    forgotten: Forgotten,
    given: Given,
}

impl Appends {
    fn new(table: TableId, from: u64, carried: Keys, hour: i32, given: Given) -> Self {
        Self { table, from, carried, hour, keys: Vec::new(), cells: Vec::new(), opaques: Vec::new(), lists: Vec::new(), forgotten: Forgotten::default(), given }
    }

    fn named(&self) -> bool {
        !self.keys.is_empty() || !self.forgotten.is_empty()
    }

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

pub struct Rows<'a> {
    store: &'a Store,
    lent: Vec<Lent>,
    logs: Vec<Appends>,
    names: Names,
    opaques: Vec<Option<Vec<u8>>>,
    lists: Vec<Option<Vec<u32>>>,
    held: Owner,
    statuses: Vec<(usize, Status)>,
    fault: Option<io::Error>,
    refused: Option<String>,
}

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
        }
    }

    fn lent_at(&mut self, table: TableId, set: bool) -> Option<usize> {
        let index = table.index();
        if let Some(at) = self.lent.iter().position(|lent| lent.table() == index) {
            return Some(at);
        }
        if self.logs.iter().any(|log| log.table == table) {
            let what = if set { "set" } else { "deleted" };
            self.refused.get_or_insert_with(|| format!("it {what} a row of log table {table:?}: a log's rows are appended and forgotten"));
            return None;
        }
        self.fault.get_or_insert_with(|| invalid(format!("A writer names {table:?}, which it does not write")));
        None
    }

    pub fn text(&mut self, text: &[u8]) -> Value {
        match self.names.id(text) {
            Ok(id) => Value::Bytes(id),
            Err(error) => {
                self.fault.get_or_insert(error);
                Value::Unrecorded
            }
        }
    }

    pub fn opaque(&mut self, bytes: Vec<u8>) -> Value {
        self.opaques.push(Some(bytes));
        Value::Opaque((self.opaques.len() - 1) as u32)
    }

    pub fn list(&mut self, values: Vec<u32>) -> Value {
        self.lists.push(Some(values));
        Value::U32List((self.lists.len() - 1) as u32)
    }

    pub fn set(&mut self, table: TableId, key: u128, cells: Vec<Value>) {
        if self.fault.is_some() || self.refused.is_some() {
            return;
        }
        let Some(at) = self.lent_at(table, true) else { return };
        let lent = &mut self.lent[at];
        if let Err(error) = check(table, lent.kinds(), &cells, &self.opaques, &self.lists) {
            self.fault = Some(error);
            return;
        }
        let (opaques, lists) = (&self.opaques, &self.lists);
        let set = {
            let _label = labels::enter(self.held, labels::Kind::Operator);
            lent.set(key, cells.iter().map(|cell| cell.cell(opaques, lists)))
        };
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

    pub fn delete(&mut self, table: TableId, key: u128) {
        if let Some(at) = self.lent_at(table, false) {
            let _label = labels::enter(self.held, labels::Kind::Operator);
            self.lent[at].delete(key);
        }
    }

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

    pub fn forget(&mut self, table: TableId, ordinal: u64) {
        self.forget_run(table, ordinal, 1);
    }

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

    pub fn table(&self, table: TableId) -> TableView<'_> {
        match self.lent.iter().find(|lent| lent.table() == table.index()) {
            Some(lent) => lent.view(self.store.strings()),
            None => self.store.view(table.index()),
        }
    }

    pub fn status(&mut self, table: TableId, status: Status) {
        let index = table.index();
        match self.statuses.iter_mut().find(|(held, _)| *held == index) {
            Some((_, held)) => *held = status,
            None => self.statuses.push((index, status)),
        }
    }

    pub fn unavailable(&mut self, table: TableId, reason: &[u8]) {
        if let Value::Bytes(reason) = self.text(reason) {
            self.status(table, Status::Unavailable { reason });
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl<'a> Rows<'a> {
    pub fn lending(store: &'a Store, lent: Vec<Lent>) -> Self {
        Self::new(store, lent, Vec::new(), Names::default(), Owner::RUN)
    }

    pub(crate) fn check_writes(self) -> io::Result<()> {
        match (self.fault, self.refused) {
            (Some(error), _) => Err(error),
            (None, Some(reason)) => Err(invalid(reason)),
            (None, None) => Ok(()),
        }
    }

    pub fn keys(&self, table: TableId) -> (Vec<u128>, Vec<u128>) {
        self.lent.iter()
            .find(|lent| lent.table() == table.index())
            .map_or_else(Default::default, Lent::pending_keys)
    }
}

pub trait Writer: Send {
    fn name(&self) -> &'static str;
    fn version(&self) -> u32;
    fn accepts_version(&self, version: u32) -> bool {
        version == self.version()
    }
    fn tables(&self) -> &'static [Output];
    fn reads(&self) -> &'static [Read];
    fn hourly(&self) -> bool {
        false
    }
    fn strings(&self) -> &'static [&'static str] {
        &[]
    }
    fn definitions(&self) -> &'static [Uses] {
        &[]
    }
    fn start(&mut self);
    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()>;
    fn save(&self, blob: &mut Bytes) -> io::Result<()>;
    fn load(&mut self, blob: &[u8]) -> io::Result<()>;
    fn prime(&mut self, hour: &Hour<'_>) {
        let _ = hour;
    }
}

pub struct Writers {
    writers: Vec<Box<dyn Writer>>,
    withheld: Vec<Option<String>>,
    ran: Vec<bool>,
    names: Names,
    spent: Vec<u64>,
    checked: bool,
    definitions: Definitions,
    reader: definitions::Reader,
    owners: Vec<Owner>,
    held: Owner,
    given: Vec<Vec<Given>>,
    format: u16,
}

fn owner(writer: &dyn Writer) -> Owner {
    if labels::ENABLED { labels::owner(&format!("writer: {}", writer.name())) } else { Owner::RUN }
}

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
            format: replay_data::FORMAT_VERSION,
        }
    }

    pub fn set_format(&mut self, format: u16) {
        self.format = format;
    }

    fn label(&self, at: usize) -> Scope {
        labels::enter(self.owners[at], labels::Kind::Operator)
    }

    fn store_label(&self) -> Scope {
        labels::enter(self.held, labels::Kind::Operator)
    }

    pub fn wanted(&self) -> Vec<Definition> {
        let mut wanted: Vec<Definition> = Vec::new();
        for uses in self.writers.iter().flat_map(|writer| writer.definitions()) {
            if !wanted.contains(&uses.definition) {
                wanted.push(uses.definition);
            }
        }
        wanted
    }

    pub fn definitions(&self) -> &Definitions {
        &self.definitions
    }

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

    pub fn define_from(&mut self, facts: &Facts, identity: &Identity) {
        let wanted = self.wanted();
        if !wanted.is_empty() {
            let read = (self.reader)(facts, identity, &wanted);
            self.define(read);
        }
    }

    pub fn read_definitions_with(&mut self, reader: definitions::Reader) {
        self.reader = reader;
    }

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

    pub fn spent(&self) -> Vec<(&'static str, u64)> {
        self.writers.iter().map(|writer| writer.name()).zip(self.spent.iter().copied()).collect()
    }

    pub fn tables(&self) -> Vec<usize> {
        let mut tables: Vec<usize> = self
            .writers
            .iter()
            .flat_map(|writer| writer.tables().iter().map(|output| output.table().index()))
            .collect();
        tables.sort_unstable();
        tables
    }

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

    pub fn push(&mut self, writer: Box<dyn Writer>) {
        self.owners.push(owner(&*writer));
        self.given.push(fresh_given(&*writer));
        self.writers.push(writer);
        self.withheld.push(None);
        self.ran.push(false);
        self.spent.push(0);
        self.checked = false;
    }

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

    fn watched(&self, watch: Watch) -> Vec<usize> {
        self.writers.iter().flat_map(|writer| writer.reads().iter().filter(|read| read.watch == watch).map(|read| read.table.index())).collect()
    }

    fn keep(&self, store: &mut Store) {
        for table in self.watched(Watch::Dropped) {
            store.keep_previous(table);
        }
        for table in self.watched(Watch::Deleted) {
            store.keep_dropped(table);
        }
    }

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

    fn string(&mut self, store: &mut Store, text: &[u8]) -> io::Result<u32> {
        let _label = self.store_label();
        let id = self.names.id(text)?;
        self.names.pin(store)?;
        Ok(id)
    }

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
                    let seen = Hour::new(store, branch, hour, first, !self.ran[at], &strings).with_format(self.format).owning(tables).defined(&self.definitions);
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
                Some((version, ran, withheld, blob, given)) if writer.accepts_version(version) && given.len() == fresh.len() => {
                    writer.load(blob)?;
                    (withheld, ran && version == writer.version(), given)
                }
                _ => (Some(OTHER_VERSION.to_owned()), false, fresh),
            };
        }
        Ok(())
    }

    pub fn table_versions(&self) -> Vec<(usize, u32)> {
        self.writers
            .iter()
            .filter(|writer| writer.version() > 1)
            .flat_map(|writer| {
                let version = writer.version();
                writer.tables().iter().map(move |output| (output.table().index(), version))
            })
            .collect()
    }
}

impl Writers {
    pub fn prime(&mut self, store: &Store, branch: u64, hour: i32) {
        for (at, writer) in self.writers.iter_mut().enumerate() {
            if self.withheld[at].is_none() && self.ran[at] {
                let _label = labels::enter(self.owners[at], labels::Kind::Operator);
                writer.prime(&Hour::new(store, branch, hour, false, false, &[]).with_format(self.format).defined(&self.definitions));
            }
        }
    }
}

fn due(writer: &dyn Writer, store: &Store, hour: i32) -> bool {
    if writer.hourly() {
        return true;
    }
    writer.reads().iter().any(|read| moved(store, read, hour))
}

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

const OTHER_VERSION: &str = "another version of the recorder worked this out, and this version cannot take up its saved work";

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
