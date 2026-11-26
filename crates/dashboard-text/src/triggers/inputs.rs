
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    hash::{DefaultHasher, Hash, Hasher},
    sync::atomic::{AtomicU64, Ordering},
};

use replay::{campaign::QuietHours, campaign::TableFields};
use replay_data::{
    BranchId, Hour, rows::DeltaRow, rows::TableId, rows::full_mask, rows::tables,
};
use replay_data::rows::{Field, List, Opaque, RowCodec, RowsVisit, TableRows};
use replay_frame::frame::{DecodedFrame, HourFrame, OperandStatus, Rowset, TableView, ViewIdentity, kept, view_reads};
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct ReuseCounts {
    pub evaluated: u64,
    pub reused: u64,
    pub checked: u64,
    pub mismatched: u64,
}

pub fn check_reuse() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    {
        CHECKING.with(Cell::get)
    }
    #[cfg(not(any(test, feature = "test-support")))]
    {
        static CHECK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *CHECK.get_or_init(|| std::env::var_os("BATTLEPLAN_REUSE_CHECK").or_else(|| std::env::var_os("HOI4_REUSE_CHECK")).is_some())
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static CHECKING: Cell<bool> = const { Cell::new(true) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn reusing<T>(f: impl FnOnce() -> T) -> T {
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            CHECKING.with(|checking| checking.set(true));
        }
    }
    CHECKING.with(|checking| checking.set(false));
    let _restore = Restore;
    f()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Key {
    Table(TableId),
    Fields(TableId, u64),
    World(super::world::Query),
    Scopes(crate::scope::Query),
    Names(crate::scripted_text::recorded_names::Query),
    Text(crate::scripted_text::TextQuery),
    ClockSeed,
    FocusTag(String),
}

impl Key {
    pub fn dependence(&self) -> Option<Vec<TableRead>> {
        match self {
            Key::Table(table) => Some(vec![TableRead::all(*table)]),
            &Key::Fields(table, fields) => Some(vec![TableRead {
                fields,
                ..TableRead::keys(table)
            }]),
            Key::World(query) => query.dependence().map(in_world),
            Key::Scopes(query) => query.dependence().map(in_world),
            Key::Names(query) => Some(query.dependence()),
            Key::Text(query) => query.dependence(),
            Key::ClockSeed => Some(vec![TableRead::fields(TableId::Clock, &["seed"])]),
            Key::FocusTag(_) => Some(vec![
                TableRead::fields(TableId::Countries, &["canonical_slot", "tag"]),
                TableRead::fields(TableId::CountryLinks, &["canonical_slot"]),
            ]),
        }
    }

    fn table(&self) -> Option<TableId> {
        match self {
            Key::Table(table) | Key::Fields(table, _) => Some(*table),
            _ => None,
        }
    }
}

fn in_world(mut reads: Vec<TableRead>) -> Vec<TableRead> {
    reads.extend(
        [
            TableId::CountryLinks,
            TableId::Countries,
            TableId::Provinces,
            TableId::States,
        ]
        .map(TableRead::keys),
    );
    reads
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct TableRead {
    pub table: TableId,
    pub fields: u64,
    pub rows: Option<(u128, u128)>,
    pub ends: bool,
    pub owner: Option<Owner>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct Owner {
    pub slot: u32,
    pub shift: u32,
    pub by: Resolution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Resolution {
    Links,
    Countries,
}

impl Resolution {
    fn read(self) -> TableRead {
        match self {
            Self::Links => TableRead::fields(TableId::CountryLinks, &["canonical_slot"]),
            Self::Countries => TableRead::fields(TableId::Countries, &["canonical_slot"]),
        }
    }

    pub fn slots<'f>(self, frame: impl Into<HourFrame<'f>>) -> Option<Slots<'f>> {
        let frame = frame.into();
        view_reads::unrecorded(|| match self {
            Self::Links => frame
                .complete::<tables::CountryLinks>(*CANONICAL_LINK)
                .ok()
                .map(Slots::Links),
            Self::Countries => frame
                .complete::<tables::Countries>(*CANONICAL_COUNTRY)
                .ok()
                .map(Slots::Countries),
        })
    }
}

static CANONICAL_LINK: LazyLock<u64> =
    LazyLock::new(|| TableRead::fields(TableId::CountryLinks, &["canonical_slot"]).fields);
static CANONICAL_COUNTRY: LazyLock<u64> =
    LazyLock::new(|| TableRead::fields(TableId::Countries, &["canonical_slot"]).fields);

#[derive(Clone, Copy)]
pub enum Slots<'a> {
    Links(Rowset<'a, replay_data::rows::CountryLinkRow>),
    Countries(Rowset<'a, replay_data::rows::CountryRow>),
}

impl Slots<'_> {
    pub fn of(self, country: u32) -> Option<u32> {
        view_reads::unrecorded(|| match self {
            Self::Links(rows) => rows
                .get(u128::from(country))
                .ok()
                .flatten()
                .map(|row| row.canonical_slot),
            Self::Countries(rows) => rows
                .get(u128::from(country))
                .ok()
                .flatten()
                .map(|row| row.canonical_slot),
        })
    }
}

impl TableRead {
    pub fn all(table: TableId) -> Self {
        Self {
            fields: full_mask(table.schema().len()),
            ..Self::keys(table)
        }
    }

    pub fn keys(table: TableId) -> Self {
        Self {
            table,
            fields: 0,
            rows: None,
            ends: false,
            owner: None,
        }
    }

    pub fn fields(table: TableId, names: &[&str]) -> Self {
        let mut fields = 0;
        for name in names {
            match (0..64).find(|&field| table.field_name(field) == Some(*name)) {
                Some(field) => fields |= 1_u64 << field,
                None => {
                    if cfg!(debug_assertions) {
                        panic!("{table:?} has no field {name}");
                    }
                    return Self::all(table);
                }
            }
        }
        Self {
            fields,
            ..Self::keys(table)
        }
    }

    pub fn row(self, key: u128) -> Self {
        self.rows(key, key)
    }

    pub fn rows(self, first: u128, last: u128) -> Self {
        Self {
            rows: Some((first, last)),
            ..self
        }
    }

    pub fn ending(self) -> Self {
        Self {
            fields: self.fields | Self::fields(self.table, &["start_hour", "end_hour"]).fields,
            ends: true,
            ..self
        }
    }

    pub fn owned_by(self, slot: u32, shift: u32) -> Self {
        Self {
            owner: Some(Owner {
                slot,
                shift,
                by: Resolution::Links,
            }),
            ..self
        }
    }

    pub fn of_country(self, slot: u32, shift: u32) -> Self {
        Self {
            owner: Some(Owner {
                slot,
                shift,
                by: Resolution::Countries,
            }),
            ..self
        }
    }

    #[cfg(test)]
    pub(crate) fn reads_row(&self, key: u128, frame: &'_ DecodedFrame) -> bool {
        self.rows
            .is_none_or(|(first, last)| first <= key && key <= last)
            && self.owner.is_none_or(|owner| {
                owner.by.slots(frame).is_none_or(|slots| {
                    slots
                        .of(owner.country(key))
                        .is_none_or(|slot| slot == owner.slot)
                })
            })
    }
}

impl Owner {
    fn country(self, key: u128) -> u32 {
        key.checked_shr(self.shift).unwrap_or(0) as u32
    }
}

#[derive(Clone, Copy, Debug)]
pub enum TableChange<'a> {
    Same,
    Rows(&'a [DeltaRow], &'a [u128]),
    Fields(u64),
    Any,
}

pub trait TableChanges {
    fn table(&self, table: TableId) -> TableChange<'_>;
}

fn watched(fields: &[TableFields], table: TableId) -> Option<u64> {
    fields
        .iter()
        .filter(|watch| watch.table == table)
        .map(|watch| watch.fields)
        .reduce(|all, fields| all | fields)
}

pub struct BlockTables<'a> {
    watched: &'a [TableFields],
    changed: &'a [TableId],
    rebuilt: &'a [TableId],
}

impl<'a> BlockTables<'a> {
    pub fn new(watched: &'a [TableFields], changed: &'a [TableId], rebuilt: &'a [TableId]) -> Self {
        Self {
            watched,
            changed,
            rebuilt,
        }
    }
}

impl TableChanges for BlockTables<'_> {
    fn table(&self, table: TableId) -> TableChange<'_> {
        if self.changed.contains(&table) {
            TableChange::Any
        } else if !self.rebuilt.contains(&table) {
            TableChange::Same
        } else {
            watched(self.watched, table)
                .map_or(TableChange::Any, |fields| TableChange::Fields(!fields))
        }
    }
}

pub struct QuietTables<'a> {
    watched: &'a [TableFields],
    quiet: &'a QuietHours,
    hours: usize,
}

impl<'a> QuietTables<'a> {
    pub fn new(watched: &'a [TableFields], quiet: &'a QuietHours, hours: usize) -> Self {
        Self {
            watched,
            quiet,
            hours,
        }
    }
}

impl TableChanges for QuietTables<'_> {
    fn table(&self, table: TableId) -> TableChange<'_> {
        if !(0..self.hours).any(|position| self.quiet.rebuilt(position, table)) {
            TableChange::Same
        } else {
            watched(self.watched, table)
                .map_or(TableChange::Any, |fields| TableChange::Fields(!fields))
        }
    }
}

pub struct Between<'a>(kept::Between<'a>);

impl<'a> Between<'a> {
    pub fn new(then: &'a DecodedFrame, now: &'a DecodedFrame) -> Self {
        Self(kept::Between::new(then, now))
    }
}

impl TableChanges for Between<'_> {
    fn table(&self, table: TableId) -> TableChange<'_> {
        use kept::{Change, Changes};
        match self.0.table(table) {
            Change::Same => TableChange::Same,
            Change::Fields(fields) => TableChange::Fields(fields),
            Change::Rows(..) | Change::Any => TableChange::Any,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub struct Differing<'a> {
    pub before: &'a DecodedFrame,
    pub after: &'a DecodedFrame,
}

#[cfg(any(test, feature = "test-support"))]
impl TableChanges for Differing<'_> {
    fn table(&self, table: TableId) -> TableChange<'_> {
        if self.before.tables[table.index()] == self.after.tables[table.index()] {
            TableChange::Same
        } else {
            TableChange::Any
        }
    }
}

pub struct Since<'a> {
    branch: BranchId,
    from: Hour,
    to: Hour,
    rows: Option<HourFrame<'a>>,
    tables: &'a dyn TableChanges,
    asked: RefCell<std::collections::HashMap<TableRead, bool>>,
    owned: RefCell<std::collections::HashMap<TableRead, Reached>>,
}

#[derive(Default)]
struct Reached {
    slots: HashSet<u32>,
    any: bool,
}

impl<'a> Since<'a> {
    pub fn hour_before(frame: &'a DecodedFrame, tables: &'a dyn TableChanges) -> Option<Self> {
        Self::hour_before_in(HourFrame::from(frame), tables)
    }

    pub fn hour_before_in(frame: HourFrame<'a>, tables: &'a dyn TableChanges) -> Option<Self> {
        Some(Self::span_in(
            frame.branch(),
            frame.hour().checked_sub(1)?,
            frame.hour(),
            Some(frame),
            tables,
        ))
    }

    pub fn span(
        branch: BranchId,
        from: Hour,
        to: Hour,
        rows: Option<&'a DecodedFrame>,
        tables: &'a dyn TableChanges,
    ) -> Self {
        Self::span_in(branch, from, to, rows.map(HourFrame::from), tables)
    }

    pub fn span_in(
        branch: BranchId,
        from: Hour,
        to: Hour,
        rows: Option<HourFrame<'a>>,
        tables: &'a dyn TableChanges,
    ) -> Self {
        Self {
            branch,
            from,
            to,
            rows: rows.map(HourFrame::from),
            tables,
            asked: RefCell::default(),
            owned: RefCell::default(),
        }
    }

    pub fn changed(&self, read: &TableRead) -> bool {
        if let Some(&changed) = self.asked.borrow().get(read) {
            return changed;
        }
        let changed = self.reads_differently(read);
        self.asked.borrow_mut().insert(*read, changed);
        changed
    }

    fn reads_differently(&self, read: &TableRead) -> bool {
        let moved = match self.tables.table(read.table) {
            TableChange::Same => false,
            TableChange::Any => true,
            TableChange::Fields(fields) => fields & read.fields != 0,
            TableChange::Rows(changed, deleted) => {
                let (changed, deleted) = match read.rows {
                    None => (changed, deleted),
                    Some((first, last)) => {
                        let start = changed.partition_point(|row| row.key < first);
                        let end = changed.partition_point(|row| row.key <= last);
                        let gone = deleted.partition_point(|&key| key < first);
                        let gone_end = deleted.partition_point(|&key| key <= last);
                        (
                            &changed[start..end.max(start)],
                            &deleted[gone..gone_end.max(gone)],
                        )
                    }
                };
                let mut keys = deleted.iter().copied().chain(
                    changed
                        .iter()
                        .filter(|row| {
                            inserted(read.table, row) || touches(read.table, read.fields, row)
                        })
                        .map(|row| row.key),
                );
                match read.owner {
                    None => keys.next().is_some(),
                    Some(owner) => {
                        let shape = TableRead {
                            owner: Some(Owner { slot: 0, ..owner }),
                            ..*read
                        };
                        if !self.owned.borrow().contains_key(&shape) {
                            let reached = self.reached(owner, keys);
                            self.owned.borrow_mut().insert(shape, reached);
                        }
                        let owned = self.owned.borrow();
                        let reached = &owned[&shape];
                        reached.any || reached.slots.contains(&owner.slot)
                    }
                }
            }
        };
        moved
            || read
                .owner
                .is_some_and(|owner| self.changed(&owner.by.read()))
            || (read.ends
                && self
                    .rows
                    .is_none_or(|rows| ended(rows, read, self.from, self.to)))
    }

    fn reached(&self, owner: Owner, mut keys: impl Iterator<Item = u128>) -> Reached {
        let Some(slots) = self.rows.and_then(|frame| owner.by.slots(frame)) else {
            return Reached {
                slots: HashSet::new(),
                any: keys.next().is_some(),
            };
        };
        let mut reached = Reached::default();
        for key in keys {
            match slots.of(owner.country(key)) {
                Some(slot) => {
                    reached.slots.insert(slot);
                }
                None => reached.any = true,
            }
        }
        reached
    }
}

fn inserted(table: TableId, row: &DeltaRow) -> bool {
    row.mask == full_mask(row.recorded_fields.min(table.schema().len()))
}

fn touches(table: TableId, fields: u64, row: &DeltaRow) -> bool {
    let unrecorded = if row.recorded_fields < table.schema().len() {
        u32::try_from(row.recorded_fields)
            .ok()
            .and_then(|recorded| fields.checked_shr(recorded))
            .unwrap_or(0)
    } else {
        0
    };
    row.mask & fields != 0 || unrecorded != 0
}

fn ended(frame: HourFrame<'_>, read: &TableRead, from: Hour, to: Hour) -> bool {
    fn any<T: replay_data::rows::TableSchema>(
        frame: HourFrame<'_>,
        read: &TableRead,
        span: impl Fn(&T::Row) -> (i32, i32),
        from: Hour,
        to: Hour,
    ) -> bool {
        use replay_frame::frame::OperandStatus;
        match frame.status(T::ID) {
            OperandStatus::Unavailable(_) | OperandStatus::NotRecorded(_) => return false,
            OperandStatus::WrongLayout | OperandStatus::TruncatedWrongLayout(_) => return true,
            OperandStatus::Complete | OperandStatus::Truncated(_) => {}
        }
        let mask = TableRead::fields(T::ID, &["start_hour", "end_hour"]).fields;
        let Some(rows) = frame.rows::<T>(mask) else {
            return true;
        };
        let rows = match read.rows {
            Some((first, last)) => rows.span(first, last),
            None => rows,
        };
        rows.iter().any(|row| {
            row.is_err()
                || row.is_ok_and(|row| {
                    let (start, end) = span(&row);
                    end > start && from < end && end <= to
                })
        })
    }
    view_reads::unrecorded(|| match read.table {
        TableId::WarRelations => any::<tables::WarRelations>(
            frame,
            read,
            |row| (row.start_hour, row.end_hour),
            from,
            to,
        ),
        TableId::Subjects => {
            any::<tables::Subjects>(frame, read, |row| (row.start_hour, row.end_hour), from, to)
        }
        _ => true,
    })
}

#[derive(Clone, Debug)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
enum Value {
    Digest(u64),
    View(ViewIdentity),
}

#[derive(Clone, Debug)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
struct Read {
    key: Key,
    value: Value,
    uses: Option<u64>,
}

#[derive(Clone, Debug, Default)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct Inputs {
    reads: Vec<Read>,
    footprint: Vec<TableRead>,
    at: Option<(BranchId, Hour)>,
}

thread_local! {
    static RECORDING: RefCell<Vec<Inputs>> = const { RefCell::new(Vec::new()) };
    static SUSPENDED: Cell<u32> = const { Cell::new(0) };
}

pub fn capture<T>(frame: &DecodedFrame, evaluate: impl FnOnce() -> T) -> (T, Inputs) {
    capture_in(HourFrame::from(frame), evaluate)
}

pub fn capture_in<T>(frame: HourFrame<'_>, evaluate: impl FnOnce() -> T) -> (T, Inputs) {
    RECORDING.with(|recording| recording.borrow_mut().push(Inputs::default()));
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            RECORDING.with(|recording| {
                recording.borrow_mut().pop();
            });
        }
    }
    let pop = Pop;
    let (value, views, walked) = view_reads::record_walked(evaluate);
    if let Some(decoded) = frame.decoded() {
        for (index, view) in decoded.decoded_frame().tables.iter().enumerate() {
            if views.iter().any(|read| read.is(view)) {
                record(Key::Table(TableId::ALL[index]), || {
                    Value::View(view.identity())
                });
            }
        }
    }
    for table in walked {
        record(Key::Table(table), || view_value(frame, table));
    }
    let mut inputs = RECORDING.with(|recording| {
        let mut recording = recording.borrow_mut();
        let inputs = recording.last_mut().map(std::mem::take).unwrap_or_default();
        if let Some(outer) = recording.iter_mut().rev().nth(1) {
            for read in &inputs.reads {
                outer.note(&read.key, &read.value);
            }
        }
        inputs
    });
    drop(pop);
    inputs.at = Some((frame.branch(), frame.hour()));
    (value, inputs)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Handoff {
    inputs: bool,
    views: bool,
}

pub(crate) struct Carried {
    inputs: Option<Inputs>,
    views: Vec<ViewIdentity>,
    walked: Vec<TableId>,
}

pub(crate) fn handoff() -> Handoff {
    Handoff {
        inputs: recording(),
        views: view_reads::recording(),
    }
}

impl Handoff {
    pub(crate) fn run<T>(self, compute: impl FnOnce() -> T) -> (T, Carried) {
        if self.inputs {
            RECORDING.with(|recording| recording.borrow_mut().push(Inputs::default()));
        }
        let (value, views, walked) = if self.views {
            view_reads::record_walked(compute)
        } else {
            (compute(), Vec::new(), Vec::new())
        };
        let inputs = self
            .inputs
            .then(|| RECORDING.with(|recording| recording.borrow_mut().pop().unwrap_or_default()));
        (
            value,
            Carried {
                inputs,
                views,
                walked,
            },
        )
    }
}

impl Carried {
    pub(crate) fn credit(self) {
        if let Some(inputs) = self.inputs {
            RECORDING.with(|recording| {
                if let Some(outer) = recording.borrow_mut().last_mut() {
                    for read in &inputs.reads {
                        outer.note(&read.key, &read.value);
                    }
                }
            });
        }
        view_reads::credit(&self.views);
        view_reads::credit_walked(&self.walked);
    }
}

pub fn query<V: Hash>(key: impl FnOnce() -> Key, compute: impl FnOnce() -> V) -> V {
    if !recording() {
        return compute();
    }
    let value = unrecorded(compute);
    record(key(), || Value::Digest(digest(&value)));
    value
}

pub fn query_typed<V: Fingerprint>(
    key: impl FnOnce() -> Key,
    compute: impl FnOnce() -> V,
) -> V {
    if !recording() {
        return compute();
    }
    let value = unrecorded(compute);
    record(key(), || Value::Digest(fingerprint(&value)));
    value
}

fn with_resolutions(
    value: u64,
    failures: &std::collections::BTreeSet<super::refusal::Identity>,
) -> u64 {
    if failures.is_empty() {
        value
    } else {
        digest(&(value, failures))
    }
}

pub(crate) fn resolved_digest(run: impl FnOnce() -> u64) -> u64 {
    let (value, failures) = super::refusal::resolutions(run);
    with_resolutions(value, &failures)
}

pub(crate) fn query_resolved<V: Hash>(key: impl FnOnce() -> Key, compute: impl FnOnce() -> V) -> V {
    if !recording() {
        return compute();
    }
    let (value, failures) = unrecorded(|| super::refusal::resolutions(compute));
    record(key(), || {
        Value::Digest(with_resolutions(digest(&value), &failures))
    });
    value
}

pub(crate) fn query_typed_resolved<V: Fingerprint>(
    key: impl FnOnce() -> Key,
    compute: impl FnOnce() -> V,
) -> V {
    if !recording() {
        return compute();
    }
    let (value, failures) = unrecorded(|| super::refusal::resolutions(compute));
    record(key(), || {
        Value::Digest(with_resolutions(fingerprint(&value), &failures))
    });
    value
}

pub(crate) fn recording() -> bool {
    SUSPENDED.with(Cell::get) == 0 && RECORDING.with(|recording| !recording.borrow().is_empty())
}

pub fn read<V: Hash>(key: Key, value: &V) {
    if recording() {
        record(key, || Value::Digest(digest(value)));
    }
}

pub fn unrecorded<T>(compute: impl FnOnce() -> T) -> T {
    SUSPENDED.with(|suspended| suspended.set(suspended.get() + 1));
    struct Resume;
    impl Drop for Resume {
        fn drop(&mut self) {
            SUSPENDED.with(|suspended| suspended.set(suspended.get() - 1));
        }
    }
    let _resume = Resume;
    view_reads::unrecorded(compute)
}

pub fn table<'f>(frame: impl Into<replay_frame::frame::HourFrame<'f>>, table: TableId) {
    let frame = frame.into();
    if recording() {
        record(Key::Table(table), || view_value(frame, table));
    }
}

fn view_value(frame: replay_frame::frame::HourFrame<'_>, table: TableId) -> Value {
    match (frame.identity(table), frame.decoded()) {
        (Some(view), _) => Value::View(view),
        (None, Some(decoded)) => Value::Digest(fingerprint(decoded.view(table.index()))),
        (None, None) => Value::Digest(fingerprint(&frame.status(table))),
    }
}

pub fn fields<'f>(
    frame: impl Into<replay_frame::frame::HourFrame<'f>>,
    table: TableId,
    names: &[&str],
) {
    let frame = frame.into();
    if recording() {
        let fields = TableRead::fields(table, names).fields;
        record(Key::Fields(table, fields), || view_value(frame, table));
    }
}

fn record(key: Key, value: impl FnOnce() -> Value) {
    if SUSPENDED.with(Cell::get) != 0 {
        return;
    }
    RECORDING.with(|recording| {
        if let Some(inputs) = recording.borrow_mut().last_mut() {
            inputs.note(&key, &value());
        }
    });
}

pub trait Fingerprint {
    fn fingerprint(&self, into: &mut DefaultHasher);
}

pub fn fingerprint<V: Fingerprint + ?Sized>(value: &V) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.fingerprint(&mut hasher);
    hasher.finish()
}

macro_rules! hashed {
    ($($held:ty),* $(,)?) => {
        $(impl Fingerprint for $held {
            fn fingerprint(&self, into: &mut DefaultHasher) {
                Hash::hash(self, into);
            }
        })*
    };
}

hashed!(
    bool, i32, u32, u64, str, String,
    super::Failure, super::Eval, super::world::PartyLeaders, super::saved_event_targets::Selected,
    crate::scope::Target, crate::scope::Unresolved, crate::scripted_text::Object,
    content_reader_key_names::tag_aliases::TagAlias, content_reader_country_names::country_names::Operands,
);

impl Fingerprint for super::world::Invasion<'_> {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        Hash::hash(self, into);
    }
}

pub(crate) fn row_fingerprint<R: RowCodec + std::fmt::Debug>(row: &R, into: &mut DefaultHasher) {
    wire_fingerprint(row, into);
    native_fingerprint(row, into);
}

fn wire_fingerprint<R: RowCodec>(row: &R, into: &mut DefaultHasher) {
    into.write_u128(row.key());
    let walked = row.visit_wire(&mut |index, field| {
        into.write_usize(index);
        field_fingerprint(field, into)
    });
    match walked {
        Ok(()) => into.write_u8(0),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => into.write_u8(1),
        Err(_) => {
            into.write_u8(2);
            into.write_u64(UNREUSABLE.fetch_add(1, Ordering::Relaxed));
        }
    }
}

static UNREUSABLE: AtomicU64 = AtomicU64::new(0);

fn native_fingerprint<V: std::fmt::Debug + ?Sized>(value: &V, into: &mut DefaultHasher) {
    struct Writing<'h>(&'h mut DefaultHasher);
    impl std::fmt::Write for Writing<'_> {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            self.0.write(text.as_bytes());
            Ok(())
        }
    }
    let _ = std::fmt::Write::write_fmt(&mut Writing(into), format_args!("{value:?}"));
}

fn list_fingerprint<T: Hash>(list: List<'_, T>, into: &mut DefaultHasher) {
    match list {
        List::Native(values) => {
            into.write_u8(0);
            values.hash(into);
        }
        List::Encoded(bytes) => {
            into.write_u8(1);
            bytes.hash(into);
        }
    }
}

fn field_fingerprint(field: Field<'_>, into: &mut DefaultHasher) -> std::io::Result<()> {
    match field {
        Field::Empty => into.write_u8(0),
        Field::NotRecorded => into.write_u8(1),
        Field::U8(value) => (2_u8, value).hash(into),
        Field::I8(value) => (3_u8, value).hash(into),
        Field::U16(value) => (4_u8, value).hash(into),
        Field::I16(value) => (5_u8, value).hash(into),
        Field::U32(value) => (6_u8, value).hash(into),
        Field::I32(value) => (7_u8, value).hash(into),
        Field::U64(value) => (8_u8, value).hash(into),
        Field::I64(value) => (9_u8, value).hash(into),
        Field::Bool(value) => (10_u8, value).hash(into),
        Field::OptionalI32(value) => (11_u8, value).hash(into),
        Field::OptionalU32(value) => (12_u8, value).hash(into),
        Field::OptionalI64(value) => (13_u8, value).hash(into),
        Field::OptionalU64(value) => (14_u8, value).hash(into),
        Field::Bytes(bytes) => (15_u8, bytes).hash(into),
        Field::U32List(list) => {
            into.write_u8(16);
            list_fingerprint(list, into);
        }
        Field::U64List(list) => {
            into.write_u8(17);
            list_fingerprint(list, into);
        }
        Field::Opaque(Opaque::Encoded(bytes, version)) => (18_u8, version, bytes).hash(into),
        Field::Opaque(Opaque::Native(value)) => {
            let mut encoded = Vec::new();
            value.encode_value(&mut encoded)?;
            (19_u8, encoded).hash(into);
        }
    }
    Ok(())
}

impl Fingerprint for replay_data::rows::IndustrialOrganisationFlag {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        (self.token, self.set_hour, self.value, self.remaining_days).hash(into);
    }
}

impl Fingerprint for replay_data::rows::IndustrialOrganisationTask {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        self.technology.as_ref().map(|technology| (technology.country, technology.handle, technology.index, technology.token)).hash(into);
        self.line.hash(into);
    }
}

macro_rules! rows {
    ($($row:ty),* $(,)?) => {
        $(impl Fingerprint for $row {
            fn fingerprint(&self, into: &mut DefaultHasher) {
                row_fingerprint(self, into);
            }
        })*
    };
}

rows!(
    replay_data::rows::GlobalSavedEventTargetRow, replay_data::rows::DivisionStatusRow, replay_data::rows::IndustrialOrganisationRow,
    replay_data::rows::IndustrialOrganisationDetailsRow, replay_data::rows::CountryCounterInputRow, replay_data::rows::UnitLeaderRow,
    replay_data::rows::CharacterRow, replay_data::rows::CountryAppearanceRow,
);

impl<T: Fingerprint + ?Sized> Fingerprint for &T {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        (**self).fingerprint(into);
    }
}

impl<T: Fingerprint> Fingerprint for Option<T> {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        match self {
            None => into.write_u8(0),
            Some(value) => {
                into.write_u8(1);
                value.fingerprint(into);
            }
        }
    }
}

impl<T: Fingerprint, E: Fingerprint> Fingerprint for Result<T, E> {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        match self {
            Ok(value) => {
                into.write_u8(0);
                value.fingerprint(into);
            }
            Err(error) => {
                into.write_u8(1);
                error.fingerprint(into);
            }
        }
    }
}

impl<T: Fingerprint> Fingerprint for [T] {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        into.write_usize(self.len());
        self.iter().for_each(|value| value.fingerprint(into));
    }
}

impl<T: Fingerprint> Fingerprint for Vec<T> {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        self.as_slice().fingerprint(into);
    }
}

impl<A: Fingerprint, B: Fingerprint> Fingerprint for (A, B) {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        self.0.fingerprint(into);
        self.1.fingerprint(into);
    }
}

impl<K: Fingerprint, V: Fingerprint> Fingerprint for std::collections::BTreeMap<K, V> {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        into.write_usize(self.len());
        for (key, value) in self {
            key.fingerprint(into);
            value.fingerprint(into);
        }
    }
}

impl<T: Fingerprint + ToOwned + ?Sized> Fingerprint for std::borrow::Cow<'_, T> {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        (**self).fingerprint(into);
    }
}

impl Fingerprint for OperandStatus<'_> {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        std::mem::discriminant(self).hash(into);
        match *self {
            OperandStatus::Complete | OperandStatus::WrongLayout => {}
            OperandStatus::TruncatedWrongLayout(missing) | OperandStatus::Truncated(missing) => into.write_u32(missing),
            OperandStatus::Unavailable(reason) => reason.hash(into),
            OperandStatus::NotRecorded(mode) => mode.byte().hash(into),
        }
    }
}

struct RowsFingerprint<'h>(&'h mut DefaultHasher);

impl RowsVisit for RowsFingerprint<'_> {
    type Output = ();
    fn rows<R: RowCodec>(self, rows: &[R]) {
        self.0.write_usize(rows.len());
        rows.iter().for_each(|row| wire_fingerprint(row, self.0));
    }
}

impl Fingerprint for TableRows {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        std::mem::discriminant(self).hash(into);
        self.visit(RowsFingerprint(into));
        native_fingerprint(self, into);
    }
}

impl Fingerprint for TableView {
    fn fingerprint(&self, into: &mut DefaultHasher) {
        std::mem::discriminant(self).hash(into);
        match self {
            TableView::Complete(rows) => rows.fingerprint(into),
            TableView::Truncated { rows, missing } => {
                rows.fingerprint(into);
                into.write_u32(*missing);
            }
            TableView::Unavailable { reason } => reason.hash(into),
            TableView::NotRecorded { mode } => mode.byte().hash(into),
        }
    }
}

pub fn digest<V: Hash + ?Sized>(value: &V) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

static MISSED: AtomicU64 = AtomicU64::new(0);

pub fn missed_dependences() -> u64 {
    MISSED.load(Ordering::Relaxed)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static LOCAL_MISSED: Cell<u64> = const { Cell::new(0) };
    static EXPECTING_MISSED: Cell<bool> = const { Cell::new(false) };
    static DECLARED: RefCell<Option<Declaration>> = const { RefCell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
type Declaration = Box<dyn Fn(&Key) -> Option<Option<Vec<TableRead>>>>;

pub fn missed_reuse(evaluation: &dyn std::fmt::Debug, frame: &DecodedFrame) {
    missed(evaluation, frame.hour);
}

pub fn missed_reuse_at(evaluation: &dyn std::fmt::Debug, hour: Hour) {
    missed(evaluation, hour);
}

fn missed(key: &dyn std::fmt::Debug, hour: Hour) {
    MISSED.fetch_add(1, Ordering::Relaxed);
    #[cfg(any(test, feature = "test-support"))]
    LOCAL_MISSED.with(|missed| missed.set(missed.get().wrapping_add(1)));
    #[cfg(any(test, feature = "test-support"))]
    if !EXPECTING_MISSED.with(Cell::get) {
        panic!(
            "{key:?} read differently at hour {hour} although its declared dependence did not change"
        );
    }
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = (key, hour);
}

#[cfg(any(test, feature = "test-support"))]
pub fn expecting_missed<T>(f: impl FnOnce() -> T) -> (T, u64) {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            EXPECTING_MISSED.with(|expecting| expecting.set(self.0));
        }
    }
    let _restore = Restore(EXPECTING_MISSED.with(|expecting| expecting.replace(true)));
    let before = LOCAL_MISSED.with(Cell::get);
    let value = f();
    (value, LOCAL_MISSED.with(Cell::get).wrapping_sub(before))
}

#[cfg(any(test, feature = "test-support"))]
pub fn declaring<T>(
    declare: impl Fn(&Key) -> Option<Option<Vec<TableRead>>> + 'static,
    f: impl FnOnce() -> T,
) -> T {
    struct Restore(Option<Declaration>);
    impl Drop for Restore {
        fn drop(&mut self) {
            DECLARED.with(|declared| *declared.borrow_mut() = self.0.take());
        }
    }
    let previous = DECLARED.with(|declared| declared.borrow_mut().replace(Box::new(declare)));
    let _restore = Restore(previous);
    f()
}

fn dependence(key: &Key) -> Option<Vec<TableRead>> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(declared) =
        DECLARED.with(|declared| declared.borrow().as_ref().and_then(|f| f(key)))
    {
        return declared;
    }
    key.dependence()
}

impl Inputs {
    fn note(&mut self, key: &Key, value: &Value) {
        if self.reads.iter().any(|read| read.key == *key) {
            return;
        }
        let uses = dependence(key).map(|reads| {
            reads
                .iter()
                .fold(0_u64, |uses, read| uses | self.footprint_bit(read))
        });
        self.reads.push(Read {
            key: key.clone(),
            value: value.clone(),
            uses,
        });
    }

    fn footprint_bit(&mut self, read: &TableRead) -> u64 {
        let position = match self.footprint.iter().position(|held| held == read) {
            Some(position) => position,
            None => {
                self.footprint.push(*read);
                self.footprint.len() - 1
            }
        };
        1_u64 << position.min(63)
    }

    fn follows(&self, since: &Since<'_>) -> bool {
        self.at == Some((since.branch, since.from))
    }

    fn changed(&self, since: &Since<'_>) -> u64 {
        self.footprint
            .iter()
            .enumerate()
            .filter(|(_, read)| since.changed(read))
            .fold(0, |changed, (position, _)| {
                changed | (1_u64 << position.min(63))
            })
    }

    pub fn footprint(&self) -> Option<&[TableRead]> {
        self.reads
            .iter()
            .all(|read| read.uses.is_some())
            .then_some(self.footprint.as_slice())
    }

    pub fn unchanged(&self, since: &Since<'_>) -> bool {
        self.follows(since)
            && self.reads.iter().all(|read| read.uses.is_some())
            && self.changed(since) == 0
    }

    pub fn advance(&mut self, since: &Since<'_>) -> bool {
        let unchanged = self.unchanged(since);
        if unchanged {
            self.at = Some((since.branch, since.to));
        }
        unchanged
    }

    pub fn confirm(
        &mut self,
        frame: &DecodedFrame,
        reread: impl FnMut(&Key) -> Option<u64>,
    ) -> bool {
        self.confirm_since(frame, None, reread)
    }

    pub fn confirm_since(
        &mut self,
        frame: &DecodedFrame,
        since: Option<&Since<'_>>,
        reread: impl FnMut(&Key) -> Option<u64>,
    ) -> bool {
        self.confirm_since_in(HourFrame::from(frame), since, reread)
    }

    pub fn confirm_since_in(
        &mut self,
        frame: HourFrame<'_>,
        since: Option<&Since<'_>>,
        mut reread: impl FnMut(&Key) -> Option<u64>,
    ) -> bool {
        let (branch, hour) = (frame.branch(), frame.hour());
        let changed = since
            .filter(|since| self.follows(since) && since.branch == branch && since.to == hour)
            .map(|since| self.changed(since));
        let checking = check_reuse();
        let decoded = frame.decoded().map(|frame| frame.decoded_frame());
        for read in &mut self.reads {
            let unchanged =
                changed.is_some_and(|changed| read.uses.is_some_and(|uses| uses & changed == 0));
            if unchanged {
                match (&mut read.value, read.key.table(), decoded) {
                    (Value::View(view), Some(table), Some(decoded)) => {
                        *view = decoded.tables[table.index()].identity()
                    }
                    (Value::Digest(digest), Some(table), None) => {
                        if checking
                            && !matches!(view_value(frame, table), Value::Digest(now) if now == *digest)
                        {
                            missed(&read.key, hour);
                        }
                    }
                    (Value::Digest(digest), _, _)
                        if checking && reread(&read.key) != Some(*digest) =>
                    {
                        missed(&read.key, hour);
                    }
                    _ => {}
                }
                continue;
            }
            let same = match (&read.key, &read.value, decoded) {
                (Key::Table(table) | Key::Fields(table, _), Value::View(view), Some(decoded)) => {
                    view.is(&decoded.tables[table.index()])
                }
                (Key::Table(_) | Key::Fields(..), Value::Digest(_), None) => false,
                (_, Value::Digest(digest), _) => reread(&read.key) == Some(*digest),
                _ => false,
            };
            if !same {
                return false;
            }
        }
        self.at = Some((branch, hour));
        true
    }

    #[cfg(test)]
    fn tables(&self) -> impl Iterator<Item = TableId> + '_ {
        self.reads.iter().filter_map(|read| match read.key {
            Key::Table(table) => Some(table),
            _ => None,
        })
    }

    pub fn declared(&self) -> Option<&[TableRead]> {
        self.reads
            .iter()
            .all(|read| read.uses.is_some())
            .then_some(self.footprint.as_slice())
    }

    pub fn keys(&self) -> impl Iterator<Item = &Key> + '_ {
        self.reads.iter().map(|read| &read.key)
    }
}

#[cfg(test)]
#[path = "inputs_tests.rs"]
pub(crate) mod tests;
