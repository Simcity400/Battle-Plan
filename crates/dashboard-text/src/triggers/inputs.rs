//! Verified reuse of evaluations. An evaluation (a country's dynamic focus
//! presentation, a leader's traits, a scripted text) records every value it
//! reads as an input: a key naming the read and a digest of the value read.
//! A later hour reuses the evaluation's result when every input reads the
//! same value again: the result is a function of the values read, so equal
//! reads give an equal result.
//!
//! The hour's shared state (the trigger [`super::World`], the recorded
//! scopes and names) is read only through recorded queries ([`query`]),
//! which record their arguments and the digest of their result. What a query
//! reads inside, its tables, lazily built indexes and time comparisons, is
//! not recorded: the result already holds its whole effect. Any other table
//! read during an evaluation is recorded whole ([`Key::Table`]), through
//! [`replay_frame::frame::view_reads`], or by the fields it reads ([`fields`]).
//!
//! Each input also declares what its value is a function of
//! ([`Key::dependence`]): some fields of some rows of some tables, their keys
//! and availability; in a table keyed by raw country, the rows of one
//! canonical country and those whose country does not resolve ([`Owner`]).
//! Given what changed since the hour an evaluation last
//! read equal ([`Since`]), an input none of whose declared reads changed
//! reads the same without being read again, so confirming it needs no
//! trigger world ([`Inputs::confirm_since`]). An input without a declaration
//! is always read again. Check mode reads every input so skipped again and
//! reports a declaration that missed a read ([`missed_dependences`]).

use std::{
    cell::{Cell, RefCell},
    hash::{Hash, Hasher},
    sync::atomic::{AtomicU64, Ordering},
};

use replay::{container::QuietHours, container::TableFields};
use replay_data::{
    BranchId, Hour, rows::DeltaRow, rows::RowCodec, rows::TableId, rows::full_mask, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::TableView, frame::ViewIdentity, frame::view_reads};

/// Evaluations run, reused, and (in check mode) reused but run anyway,
/// and of those, any that changed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReuseCounts {
    pub evaluated: u64,
    pub reused: u64,
    pub checked: u64,
    pub mismatched: u64,
}

/// Check mode runs every reused evaluation anyway and counts any change:
/// in tests unless one turns it off ([`reusing`]), and in preparation when
/// `BATTLE_PLAN_REUSE_CHECK` is set.
pub fn check_reuse() -> bool {
    #[cfg(any(test, feature = "test-support"))]
    {
        CHECKING.with(Cell::get)
    }
    #[cfg(not(any(test, feature = "test-support")))]
    {
        static CHECK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *CHECK.get_or_init(|| std::env::var_os("BATTLE_PLAN_REUSE_CHECK").or_else(|| std::env::var_os("HOI4_REUSE_CHECK")).is_some())
    }
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static CHECKING: Cell<bool> = const { Cell::new(true) };
}

/// Run `f` on this thread with check mode off, so reused results are
/// returned as preparation returns them.
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

/// What an evaluation read.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A whole table's view at the hour: its allocation, which changes with
    /// any of its rows, status or refusal.
    Table(TableId),
    /// Some fields (change-mask bits) of every row of a table's view: its
    /// allocation, as [`Key::Table`], which the hour's changes confirm by
    /// these fields, the row keys and availability alone ([`fields`]).
    Fields(TableId, u64),
    /// A query of the hour's trigger world.
    World(super::world::Query),
    /// A query of the hour's recorded scopes.
    Scopes(crate::scope::Query),
    /// A query of the hour's country names.
    Names(crate::scripted_text::recorded_names::Query),
    /// A scripted text's own read of the hour.
    Text(crate::scripted_text::TextQuery),
    /// The Clock's random seed, read when a random number is drawn from it.
    ClockSeed,
    /// A focus country's tag resolved to its canonical slot.
    FocusTag(String),
}

impl Key {
    /// What the value read under this key is a function of, besides the
    /// content: every table it reads, with the fields and rows it reads.
    /// None when undeclared: the key is then read again whenever an
    /// evaluation that read it is confirmed.
    pub fn dependence(&self) -> Option<Vec<TableRead>> {
        match self {
            Key::Table(table) => Some(vec![TableRead::all(*table)]),
            &Key::Fields(table, fields) => Some(vec![TableRead {
                fields,
                ..TableRead::keys(table)
            }]),
            Key::World(query) => query.dependence().map(in_world),
            Key::Scopes(query) => query.dependence().map(in_world),
            // Names and texts read the hour's tables, never its world.
            Key::Names(query) => Some(query.dependence()),
            Key::Text(query) => query.dependence(),
            Key::ClockSeed => Some(vec![TableRead::fields(TableId::Clock, &["seed"])]),
            Key::FocusTag(_) => Some(vec![
                TableRead::fields(TableId::Countries, &["canonical_slot", "tag"]),
                TableRead::fields(TableId::CountryLinks, &["canonical_slot"]),
            ]),
        }
    }

    /// The table a view-valued key names.
    fn table(&self) -> Option<TableId> {
        match self {
            Key::Table(table) | Key::Fields(table, _) => Some(*table),
            _ => None,
        }
    }
}

/// A world or scope query's reads, and the reads its world is built from:
/// the hour's trigger world exists only while its core tables are complete
/// ([`super::world::core_gap`]).
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

/// What a read's value is a function of in one table: some fields of some
/// rows. The keys of those rows and the table's availability (its status,
/// a truncated table's missing count, an unavailable table's reason)
/// always count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TableRead {
    pub table: TableId,
    /// The fields read, as change-mask bits ([`TableId::field_name`]).
    pub fields: u64,
    /// The keys of the rows read, inclusive; None for every row.
    pub rows: Option<(u128, u128)>,
    /// Whether the value also compares the hour with each row's end hour,
    /// as an ongoing relation does: it then changes when a row ends.
    pub ends: bool,
    /// Of a table keyed by raw country first, only the rows of one
    /// canonical country and the rows whose country does not resolve
    /// ([`Owner`]); None for every row.
    pub owner: Option<Owner>,
}

/// The rows of one canonical country in a table keyed by raw country
/// first: those whose key's country (`key >> shift`) the hour's
/// `CountryLinks` maps to `slot`, and those whose country it does not
/// map, which a read refuses on. Which rows these are is a function of
/// every `CountryLinks` row's slot, which such a read also reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Owner {
    pub slot: u32,
    pub shift: u32,
}

impl TableRead {
    /// Every field of every row.
    pub fn all(table: TableId) -> Self {
        Self {
            fields: full_mask(table.schema().len()),
            ..Self::keys(table)
        }
    }

    /// No field: the row keys and availability alone.
    pub fn keys(table: TableId) -> Self {
        Self {
            table,
            fields: 0,
            rows: None,
            ends: false,
            owner: None,
        }
    }

    /// The fields `names` name, of every row. A name the table lacks reads
    /// every field, so a renamed field is never missed (and fails a debug
    /// build).
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

    /// Only the row keyed `key`.
    pub fn row(self, key: u128) -> Self {
        self.rows(key, key)
    }

    /// Only the rows keyed `first` to `last`.
    pub fn rows(self, first: u128, last: u128) -> Self {
        Self {
            rows: Some((first, last)),
            ..self
        }
    }

    /// Also the hour against each row's start and end hours, which it
    /// reads.
    pub fn ending(self) -> Self {
        Self {
            fields: self.fields | Self::fields(self.table, &["start_hour", "end_hour"]).fields,
            ends: true,
            ..self
        }
    }

    /// Only the rows of canonical country `slot`, and the rows whose
    /// country does not resolve, in a table whose key holds the raw
    /// country above bit `shift` ([`Owner`]).
    pub fn owned_by(self, slot: u32, shift: u32) -> Self {
        Self {
            owner: Some(Owner { slot, shift }),
            ..self
        }
    }

    /// Whether the key range holds `key`; an owned read holds only some of
    /// those rows, by the hour's links.
    pub(crate) fn holds(&self, key: u128) -> bool {
        self.rows
            .is_none_or(|(first, last)| first <= key && key <= last)
    }

    /// Whether this read reads the row keyed `key` of `frame`: in its key
    /// range and, if owned, its country's or unresolved by `frame`'s links.
    #[cfg(test)]
    pub(crate) fn reads_row(&self, key: u128, frame: &DecodedFrame) -> bool {
        self.holds(key)
            && self.owner.is_none_or(
                |owner| match frame.complete::<tables::CountryLinks>().ok() {
                    Some(links) => owned(links, owner, key),
                    None => true,
                },
            )
    }
}

/// What an owned read also reads: every `CountryLinks` row's slot, which
/// decides the rows it reads.
fn owner_links() -> TableRead {
    TableRead::fields(TableId::CountryLinks, &["canonical_slot"])
}

/// Whether `links` (a `CountryLinks` table's rows, ascending by raw id)
/// maps a row keyed `key` to `owner`'s country or to none.
fn owned(links: &[replay_data::rows::CountryLinkRow], owner: Owner, key: u128) -> bool {
    // The raw country, the 32 bits above the shift.
    let country = key.checked_shr(owner.shift).unwrap_or(0) as u32;
    match links.binary_search_by_key(&country, |link| link.id) {
        Ok(at) => links[at].canonical_slot == owner.slot,
        Err(_) => true,
    }
}

/// How a table may differ between two hours of a branch.
#[derive(Clone, Copy, Debug)]
pub enum TableChange<'a> {
    /// The same view: nothing changed.
    Same,
    /// Changed by exactly these rows, each with the fields it changed, and
    /// these deleted keys.
    Rows(&'a [DeltaRow], &'a [u128]),
    /// Only fields among these may have changed, in any row; the row keys
    /// and availability are equal.
    Fields(u64),
    /// Anything may have changed.
    Any,
}

/// What changed in each table between two hours of a branch. Changed rows
/// and deleted keys ascend by key, as a decoded delta's do.
pub trait TableChanges {
    fn table(&self, table: TableId) -> TableChange<'_>;
}

/// The fields a block reader watches in `table`, if it watches it.
fn watched(fields: &[TableFields], table: TableId) -> Option<u64> {
    fields
        .iter()
        .filter(|watch| watch.table == table)
        .map(|watch| watch.fields)
        .reduce(|all, fields| all | fields)
}

/// The [`TableChanges`] of a `BlockChange::Changed` hour since the hour
/// before it, for a reader watching `watched`: a changed table may have
/// changed anywhere; a rebuilt watched table only in fields it does not
/// watch; any other table not at all.
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

/// The [`TableChanges`] of a `BlockChange::Quiet` span's first `hours`
/// hours since the changed hour before it, for a reader watching `watched`.
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

/// The [`TableChanges`] between two frames of a branch, for tests: a table
/// whose rows or availability differ may have changed anywhere.
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

/// What changed on a branch from hour `from` to hour `to`, and a frame
/// holding, of every table these changes leave unchanged, its rows at `to`:
/// the frame at `to` or at `from`.
pub struct Since<'a> {
    branch: BranchId,
    from: Hour,
    to: Hour,
    rows: Option<&'a DecodedFrame>,
    tables: &'a dyn TableChanges,
    /// Each read asked about, and whether it changed: the evaluations an
    /// hour confirms share most of their reads.
    asked: RefCell<std::collections::HashMap<TableRead, bool>>,
}

impl<'a> Since<'a> {
    /// `frame`'s `tables` changes since the hour before it on its branch.
    pub fn hour_before(frame: &'a DecodedFrame, tables: &'a dyn TableChanges) -> Option<Self> {
        Some(Self {
            branch: frame.branch,
            from: frame.hour.checked_sub(1)?,
            to: frame.hour,
            rows: Some(frame),
            tables,
            asked: RefCell::default(),
        })
    }

    /// `tables` changes on `branch` from `from` to `to`, read without a
    /// frame at `to` (a quiet span); `rows` is the frame at `from`, if held.
    pub fn span(
        branch: BranchId,
        from: Hour,
        to: Hour,
        rows: Option<&'a DecodedFrame>,
        tables: &'a dyn TableChanges,
    ) -> Self {
        Self {
            branch,
            from,
            to,
            rows,
            tables,
            asked: RefCell::default(),
        }
    }

    /// Whether `read` may read differently at `to` than at `from`.
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
                // The rows read, found by key among the ascending changes.
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
                    Some(owner) => self.owned_moved(owner, keys),
                }
            }
        };
        // An unmoved read's rows hold the same keys, start and end hours
        // in `rows` as at `to`; an owned read's rows are its country's while
        // the links are the same.
        moved
            || (read.owner.is_some() && self.changed(&owner_links()))
            || (read.ends
                && self
                    .rows
                    .is_none_or(|rows| ended(rows, read, self.from, self.to)))
    }

    /// Whether a row keyed by one of `keys` (changed or deleted) is one of
    /// `owner`'s or names no country, by the links in `rows`, which equal
    /// the links at `to` whenever an owned read can be unchanged. Without
    /// them, any row may be.
    fn owned_moved(&self, owner: Owner, mut keys: impl Iterator<Item = u128>) -> bool {
        let links = self.rows.and_then(|frame| {
            view_reads::unrecorded(|| match &*frame.tables[TableId::CountryLinks.index()] {
                TableView::Complete(rows) => rows.rows::<tables::CountryLinks>(),
                // A truncated table may lack the row that resolves a key.
                TableView::Truncated { .. }
                | TableView::Unavailable { .. }
                | TableView::NotRecorded { .. } => None,
            })
        });
        match links {
            Some(links) => keys.any(|key| owned(links, owner, key)),
            None => keys.next().is_some(),
        }
    }
}

/// Whether a delta row may insert its row: an inserted row carries every
/// field it records.
fn inserted(table: TableId, row: &DeltaRow) -> bool {
    row.mask == full_mask(row.recorded_fields.min(table.schema().len()))
}

/// Whether a delta row may change one of `fields`: one in its mask, or one
/// at or past the fields an older format recorded, which a row it names
/// takes as not recorded.
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

/// Whether one of `read`'s rows in `frame` ends after `from` and by `to`,
/// so a relation ongoing at `from` is not at `to`
/// ([`crate::triggers::ongoing`]). Every other table may.
fn ended(frame: &DecodedFrame, read: &TableRead, from: Hour, to: Hour) -> bool {
    fn any<T: replay_data::rows::TableSchema>(
        view: &TableView,
        read: &TableRead,
        span: impl Fn(&T::Row) -> (i32, i32),
        from: Hour,
        to: Hour,
    ) -> bool {
        let rows = match view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows,
            TableView::Unavailable { .. } | TableView::NotRecorded { .. } => return false,
        };
        rows.rows::<T>().is_none_or(|rows| {
            rows.iter().any(|row| {
                let (start, end) = span(row);
                read.holds(row.key()) && end > start && from < end && end <= to
            })
        })
    }
    view_reads::unrecorded(|| {
        let view = &*frame.tables[read.table.index()];
        match read.table {
            TableId::WarRelations => any::<tables::WarRelations>(
                view,
                read,
                |row| (row.start_hour, row.end_hour),
                from,
                to,
            ),
            TableId::Subjects => {
                any::<tables::Subjects>(view, read, |row| (row.start_hour, row.end_hour), from, to)
            }
            _ => true,
        }
    })
}

/// The value an input read: a digest, or a table view's identity, which
/// keeps the view's allocation reserved so no later view can share it.
#[derive(Clone, Debug)]
enum Value {
    Digest(u64),
    View(ViewIdentity),
}

/// One input: its key, the value read, and the footprint entries its value
/// is a function of, a bit each (the last bit also for every entry past
/// it); None when its key declares no dependence.
#[derive(Clone, Debug)]
struct Read {
    key: Key,
    value: Value,
    uses: Option<u64>,
}

/// The reads of one evaluation, each key once, in first-read order.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    reads: Vec<Read>,
    /// Every table read the reads declare, each once.
    footprint: Vec<TableRead>,
    /// The branch and hour at which every input last read its value.
    at: Option<(BranchId, Hour)>,
}

thread_local! {
    /// The evaluations recording on this thread, innermost last.
    static RECORDING: RefCell<Vec<Inputs>> = const { RefCell::new(Vec::new()) };
    /// Queries and shared builds running inside an evaluation.
    static SUSPENDED: Cell<u32> = const { Cell::new(0) };
}

/// Run `evaluate`, recording its reads: its queries, and every table view
/// of `frame` it reads outside a query.
pub fn capture<T>(frame: &DecodedFrame, evaluate: impl FnOnce() -> T) -> (T, Inputs) {
    RECORDING.with(|recording| recording.borrow_mut().push(Inputs::default()));
    // Pop even when `evaluate` unwinds, so a later capture starts clean.
    struct Pop;
    impl Drop for Pop {
        fn drop(&mut self) {
            RECORDING.with(|recording| {
                recording.borrow_mut().pop();
            });
        }
    }
    let pop = Pop;
    let (value, views) = view_reads::record(evaluate);
    for (index, view) in frame.tables.iter().enumerate() {
        if views.iter().any(|read| read.is(view)) {
            record(Key::Table(TableId::ALL[index]), || {
                Value::View(view.identity())
            });
        }
    }
    let mut inputs = RECORDING.with(|recording| {
        let mut recording = recording.borrow_mut();
        let inputs = recording.last_mut().map(std::mem::take).unwrap_or_default();
        // An enclosing evaluation read everything this one read.
        if let Some(outer) = recording.iter_mut().rev().nth(1) {
            for read in &inputs.reads {
                outer.note(&read.key, &read.value);
            }
        }
        inputs
    });
    drop(pop);
    inputs.at = Some((frame.branch, frame.hour));
    (value, inputs)
}

/// What this thread records, for a helper thread that continues the same
/// evaluation: [`Handoff::run`] there, then [`Carried::credit`] here.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Handoff {
    inputs: bool,
    views: bool,
}

/// The reads a helper thread made for its caller's recordings.
pub(crate) struct Carried {
    inputs: Option<Inputs>,
    views: Vec<ViewIdentity>,
}

/// This thread's recording state, for a helper thread.
pub(crate) fn handoff() -> Handoff {
    Handoff {
        inputs: recording(),
        views: view_reads::recording(),
    }
}

impl Handoff {
    /// Run `compute` on the helper thread, recording what the caller records.
    pub(crate) fn run<T>(self, compute: impl FnOnce() -> T) -> (T, Carried) {
        if self.inputs {
            RECORDING.with(|recording| recording.borrow_mut().push(Inputs::default()));
        }
        let (value, views) = if self.views {
            view_reads::record(compute)
        } else {
            (compute(), Vec::new())
        };
        let inputs = self
            .inputs
            .then(|| RECORDING.with(|recording| recording.borrow_mut().pop().unwrap_or_default()));
        (value, Carried { inputs, views })
    }
}

impl Carried {
    /// Credit the helper thread's reads to this thread's recordings.
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
    }
}

/// Run a query of the hour's shared state, recording `key` and the digest of
/// its result in the evaluations on this thread; what it reads inside is not
/// recorded, since its result holds all of it.
pub fn query<V: Hash>(key: impl FnOnce() -> Key, compute: impl FnOnce() -> V) -> V {
    if !recording() {
        return compute();
    }
    let value = unrecorded(compute);
    record(key(), || Value::Digest(digest(&value)));
    value
}

/// [`query`] of a result without a `Hash`, digested through its `Debug`
/// form, which a derived `Debug` writes in full.
pub fn query_debug<V: std::fmt::Debug>(
    key: impl FnOnce() -> Key,
    compute: impl FnOnce() -> V,
) -> V {
    if !recording() {
        return compute();
    }
    let value = unrecorded(compute);
    record(key(), || Value::Digest(digest_debug(&value)));
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

/// A filtered failed join is part of the answer even when its value is unchanged.
/// Include it when confirming reuse, so a formerly proved nonmatch cannot be
/// reused after a raw reference stops resolving.
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

pub(crate) fn query_debug_resolved<V: std::fmt::Debug>(
    key: impl FnOnce() -> Key,
    compute: impl FnOnce() -> V,
) -> V {
    if !recording() {
        return compute();
    }
    let (value, failures) = unrecorded(|| super::refusal::resolutions(compute));
    record(key(), || {
        Value::Digest(with_resolutions(digest_debug(&value), &failures))
    });
    value
}

/// Whether an evaluation on this thread records reads here.
fn recording() -> bool {
    SUSPENDED.with(Cell::get) == 0 && RECORDING.with(|recording| !recording.borrow().is_empty())
}

/// Record that the evaluations on this thread read `value` under `key`.
pub fn read<V: Hash>(key: Key, value: &V) {
    if recording() {
        record(key, || Value::Digest(digest(value)));
    }
}

/// Run `compute` without recording anything it reads: building the hour's
/// shared state, whose every later use is a recorded query.
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

/// Record a read of `table`'s whole view in `frame`, for state built from
/// the whole table that an evaluation reads directly.
pub fn table(frame: &dyn replay_frame::frame::DecodedSource, table: TableId) {
    if recording() {
        record(Key::Table(table), || {
            frame
                .identity(table.index())
                .map(Value::View)
                .unwrap_or_else(|| Value::Digest(digest_debug(frame.view(table.index()))))
        });
    }
}

/// Record a read of the fields `names` name in every row of `table`'s view
/// in `frame`, for state built from those fields of the whole table that an
/// evaluation reads directly ([`Key::Fields`]).
pub fn fields(frame: &dyn replay_frame::frame::DecodedSource, table: TableId, names: &[&str]) {
    if recording() {
        let fields = TableRead::fields(table, names).fields;
        record(Key::Fields(table, fields), || {
            frame
                .identity(table.index())
                .map(Value::View)
                .unwrap_or_else(|| Value::Digest(digest_debug(frame.view(table.index()))))
        });
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

/// [`digest`] of `value`'s `Debug` form.
pub fn digest_debug<V: std::fmt::Debug + ?Sized>(value: &V) -> u64 {
    struct Digesting(std::hash::DefaultHasher);
    impl std::fmt::Write for Digesting {
        fn write_str(&mut self, text: &str) -> std::fmt::Result {
            self.0.write(text.as_bytes());
            Ok(())
        }
    }
    let mut hasher = Digesting(std::hash::DefaultHasher::new());
    std::fmt::Write::write_fmt(&mut hasher, format_args!("{value:?}")).expect("digest write");
    hasher.0.finish()
}

/// A digest of `value`; equal values have equal digests.
pub fn digest<V: Hash + ?Sized>(value: &V) -> u64 {
    // Fixed keys, so digests compare across hours.
    let mut hasher = std::hash::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// Inputs skipped as unchanged whose value check mode found changed: each
/// a declared dependence that missed a read.
static MISSED: AtomicU64 = AtomicU64::new(0);

/// How many inputs skipped by their declared dependence check mode found
/// changed since this process started; any is a declaration that misses a
/// read.
pub fn missed_dependences() -> u64 {
    MISSED.load(Ordering::Relaxed)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// This thread's misses, independent of concurrently running tests.
    static LOCAL_MISSED: Cell<u64> = const { Cell::new(0) };
    /// Whether a missed dependence is expected here rather than a failure.
    static EXPECTING_MISSED: Cell<bool> = const { Cell::new(false) };
    /// A test's own declarations, before each key's.
    static DECLARED: RefCell<Option<Declaration>> = const { RefCell::new(None) };
}

/// A test's declaration of a key's dependence: Some to replace the key's
/// own.
#[cfg(any(test, feature = "test-support"))]
type Declaration = Box<dyn Fn(&Key) -> Option<Option<Vec<TableRead>>>>;

fn missed(key: &Key, frame: &DecodedFrame) {
    MISSED.fetch_add(1, Ordering::Relaxed);
    #[cfg(any(test, feature = "test-support"))]
    LOCAL_MISSED.with(|missed| missed.set(missed.get().wrapping_add(1)));
    #[cfg(any(test, feature = "test-support"))]
    if !EXPECTING_MISSED.with(Cell::get) {
        panic!(
            "{key:?} read differently at hour {} although its declared dependence did not change",
            frame.hour
        );
    }
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = (key, frame);
}

/// Run `f` on this thread with a missed dependence counted rather than
/// failing the test, returning how many were missed.
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

/// Run `f` on this thread with `declare` deciding each recorded key's
/// dependence where it returns Some: a test of a wrong declaration.
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

/// `key`'s dependence, or a test's declaration of it.
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
        // The first read decided the evaluation: a key reads one value.
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

    /// The bit of `read`'s footprint entry, added if new.
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

    /// Whether `since` starts where every input last read its value.
    fn follows(&self, since: &Since<'_>) -> bool {
        self.at == Some((since.branch, since.from))
    }

    /// The footprint entries `since` may have changed, a bit each.
    fn changed(&self, since: &Since<'_>) -> u64 {
        self.footprint
            .iter()
            .enumerate()
            .filter(|(_, read)| since.changed(read))
            .fold(0, |changed, (position, _)| {
                changed | (1_u64 << position.min(63))
            })
    }

    /// Whether every input reads its recorded value at `since`'s end without
    /// being read again: each declares its dependence, none of which `since`
    /// changed, and `since` starts where they last read it. Confirming such
    /// an evaluation needs no trigger world.
    pub fn unchanged(&self, since: &Since<'_>) -> bool {
        self.follows(since)
            && self.reads.iter().all(|read| read.uses.is_some())
            && self.changed(since) == 0
    }

    /// [`Self::unchanged`], then the inputs read their values at `since`'s
    /// end: for a span read without frames (a quiet span).
    pub fn advance(&mut self, since: &Since<'_>) -> bool {
        let unchanged = self.unchanged(since);
        if unchanged {
            self.at = Some((since.branch, since.to));
        }
        unchanged
    }

    /// Whether every input reads its recorded value again: a table is still
    /// the same view of `frame`, and `reread` gives every other key's digest
    /// now, None when it cannot read it.
    pub fn confirm(
        &mut self,
        frame: &DecodedFrame,
        reread: impl FnMut(&Key) -> Option<u64>,
    ) -> bool {
        self.confirm_since(frame, None, reread)
    }

    /// [`Self::confirm`] with what changed since the hour the inputs last
    /// read their values at, if `since` ends at `frame` and starts there: an
    /// input whose declared dependence `since` did not change reads the same
    /// without `reread` (a table's view is then this hour's). Check mode
    /// rereads it anyway and reports a value that changed
    /// ([`missed_dependences`]).
    pub fn confirm_since(
        &mut self,
        frame: &DecodedFrame,
        since: Option<&Since<'_>>,
        mut reread: impl FnMut(&Key) -> Option<u64>,
    ) -> bool {
        let changed = since
            .filter(|since| {
                self.follows(since) && since.branch == frame.branch && since.to == frame.hour
            })
            .map(|since| self.changed(since));
        let checking = check_reuse();
        for read in &mut self.reads {
            let unchanged =
                changed.is_some_and(|changed| read.uses.is_some_and(|uses| uses & changed == 0));
            if unchanged {
                match (&mut read.value, read.key.table()) {
                    (Value::View(view), Some(table)) => {
                        *view = frame.tables[table.index()].identity()
                    }
                    (Value::Digest(digest), _)
                        if checking && reread(&read.key) != Some(*digest) =>
                    {
                        missed(&read.key, frame);
                    }
                    _ => {}
                }
                continue;
            }
            let same = match (&read.key, &read.value) {
                (Key::Table(table) | Key::Fields(table, _), Value::View(view)) => {
                    view.is(&frame.tables[table.index()])
                }
                (_, Value::Digest(digest)) => reread(&read.key) == Some(*digest),
                _ => false,
            };
            if !same {
                return false;
            }
        }
        self.at = Some((frame.branch, frame.hour));
        true
    }

    /// The tables read whole.
    #[cfg(test)]
    fn tables(&self) -> impl Iterator<Item = TableId> + '_ {
        self.reads.iter().filter_map(|read| match read.key {
            Key::Table(table) => Some(table),
            _ => None,
        })
    }

    /// The keys read, in first-read order.
    pub fn keys(&self) -> impl Iterator<Item = &Key> + '_ {
        self.reads.iter().map(|read| &read.key)
    }
}

#[cfg(test)]
#[path = "inputs_tests.rs"]
pub(crate) mod tests;
