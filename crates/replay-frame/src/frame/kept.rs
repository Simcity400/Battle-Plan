//! Answers kept across hours while every row, row range, table field and derived entry they
//! read reads the same: an hour rebuilds only the answers whose reads changed, at any step.
//!
//! A computation runs inside [`record`]. Every table it reads through the frame is a read of
//! the whole table, unless the computation notes narrower reads of that table with
//! [`note_row`], [`note_range`] or [`note_table`]: then those notes are all it read of it, so
//! a computation that notes one read of a table notes every read of it. A product's own
//! derived reads are noted with [`note`].
//!
//! [`Kept`] holds the answers with their reads. An hour's [`Changes`] say what changed since
//! the hour the answers were kept at: a walk's change facts, or two held frames compared
//! ([`Between`]). An indexed [`Kept`] finds the answers those changes reach through
//! [`Dependents`], at a cost that follows what changed; a plain one checks each answer it is
//! asked for, and each read once.
mod dependents;
pub use dependents::{Dependents, Grouped};

use super::{DecodedFrame, HourFrame, SharedView, TableView, view_reads};
use crate::rows::{RowCodec, RowsPairVisit, TABLE_COUNT, TableId};
use std::{
    any::Any,
    cell::{OnceCell, RefCell},
    collections::{HashMap, HashSet},
    hash::Hash,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// One read an answer depends on. `D` is the product's own derived read.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Read<D> {
    /// The fields `fields` of the row keyed `key`, or its absence.
    Row { table: TableId, key: u128, fields: u64 },
    /// Which rows hold keys in `from..to`, and their fields `fields`.
    Range { table: TableId, from: u128, to: u128, fields: u64 },
    /// The fields `fields` of every row, and which rows there are.
    Table { table: TableId, fields: u64 },
    Derived(D),
}

impl<D> Read<D> {
    /// The table a row, range or table read reads.
    pub fn table(&self) -> Option<TableId> {
        match self {
            Self::Row { table, .. } | Self::Range { table, .. } | Self::Table { table, .. } => {
                Some(*table)
            }
            Self::Derived(_) => None,
        }
    }
}

/// What changed of a table since the hour kept answers were kept at.
#[derive(Clone, Copy, Debug)]
pub enum Change<'a> {
    Same,
    /// Only these fields, of rows that all stay.
    Fields(u64),
    /// These rows ascending by key, each with the fields it wrote (every field for one
    /// inserted), and these keys deleted.
    Rows(&'a [(u128, u64)], &'a [u128]),
    /// Anything.
    Any,
}

/// What changed since the hour kept answers were kept at, table by table.
pub trait Changes {
    /// What may have changed of `table`, as far as is known cheaply.
    fn table(&self, table: TableId) -> Change<'_>;

    /// Which rows of `table` changed, when [`Self::table`] says less, at a cost that follows
    /// the table; None when unknown.
    fn rows(&self, table: TableId) -> Option<(&[(u128, u64)], &[u128])> {
        let _ = table;
        None
    }

    /// Whether `table`'s rows keyed `from..to` read the same in `fields`, compared directly
    /// where both hours' rows are held; None when they are not.
    fn same_rows(&self, table: TableId, from: u128, to: u128, fields: u64) -> Option<bool> {
        let _ = (table, from, to, fields);
        None
    }
}

/// Whether `changed` rows or `deleted` keys reach keys `from..to` in `fields`.
fn reaches(changed: &[(u128, u64)], deleted: &[u128], from: u128, to: u128, fields: u64) -> bool {
    let start = changed.partition_point(|row| row.0 < from);
    let gone = deleted.partition_point(|&key| key < from);
    changed[start..]
        .iter()
        .take_while(|row| row.0 < to)
        .any(|row| row.1 & fields != 0 || row.1 == u64::MAX)
        || deleted.get(gone).is_some_and(|&key| key < to)
}

/// The changes between two held frames of a branch: a table's field stamps first, then its
/// rows compared, once per table.
pub struct Between<'a> {
    then: &'a DecodedFrame,
    now: &'a DecodedFrame,
    rows: [OnceCell<Option<(Vec<(u128, u64)>, Vec<u128>)>>; TABLE_COUNT],
}

impl<'a> Between<'a> {
    pub fn new(then: &'a DecodedFrame, now: &'a DecodedFrame) -> Self {
        Self {
            then,
            now,
            rows: std::array::from_fn(|_| OnceCell::new()),
        }
    }

    fn views(&self, table: TableId) -> (&'a SharedView, &'a SharedView) {
        (&self.then.tables[table.index()], &self.now.tables[table.index()])
    }
}

impl Changes for Between<'_> {
    fn table(&self, table: TableId) -> Change<'_> {
        let (then, now) = self.views(table);
        match then.changed_fields(now) {
            Some(0) => Change::Same,
            Some(fields) => Change::Fields(fields),
            None => Change::Any,
        }
    }

    fn rows(&self, table: TableId) -> Option<(&[(u128, u64)], &[u128])> {
        self.rows[table.index()]
            .get_or_init(|| {
                let (then, now) = self.views(table);
                changed_rows(table, then, now)
            })
            .as_ref()
            .map(|(changed, deleted)| (changed.as_slice(), deleted.as_slice()))
    }

    fn same_rows(&self, table: TableId, from: u128, to: u128, fields: u64) -> Option<bool> {
        let (then, now) = self.views(table);
        match (&**then, &**now) {
            (TableView::Complete(old), TableView::Complete(new)) => {
                match old.visit_pair(new, Compare { from, to, fields }) {
                    Ok(same) => Some(same),
                    Err(_) => {
                        mismatched(table);
                        Some(false)
                    }
                }
            }
            _ => None,
        }
    }
}

/// A recording on this thread's stack.
trait Frame {
    fn table_read(&mut self, read: TableRead);
    fn as_any(&mut self) -> &mut dyn Any;
}

#[derive(Clone, Copy)]
enum TableRead {
    Row(TableId, u128, u64),
    Range(TableId, u128, u128, u64),
    Table(TableId, u64),
}

impl<D: 'static> Frame for Vec<Read<D>> {
    fn table_read(&mut self, read: TableRead) {
        self.push(match read {
            TableRead::Row(table, key, fields) => Read::Row { table, key, fields },
            TableRead::Range(table, from, to, fields) => Read::Range { table, from, to, fields },
            TableRead::Table(table, fields) => Read::Table { table, fields },
        });
    }
    fn as_any(&mut self) -> &mut dyn Any {
        self
    }
}

thread_local! {
    static FRAMES: RefCell<Vec<Box<dyn Frame>>> = const { RefCell::new(Vec::new()) };
}

fn note_table_read(read: TableRead) {
    FRAMES.with(|frames| {
        if let Some(frame) = frames.borrow_mut().last_mut() {
            frame.table_read(read);
        }
    });
}

/// Notes a read of the fields `fields` of `table`'s row keyed `key`, or of its absence.
pub fn note_row(table: TableId, key: u128, fields: u64) {
    note_table_read(TableRead::Row(table, key, fields));
}

/// Notes a read of which rows of `table` hold keys in `from..to`, and of their fields `fields`.
pub fn note_range(table: TableId, from: u128, to: u128, fields: u64) {
    note_table_read(TableRead::Range(table, from, to, fields));
}

/// Notes a read of the fields `fields` of all of `table`.
pub fn note_table(table: TableId, fields: u64) {
    note_table_read(TableRead::Table(table, fields));
}

/// Notes the product's derived `read` in a recording of its kind.
pub fn note<D: 'static>(read: impl FnOnce() -> D) {
    FRAMES.with(|frames| {
        if let Some(frame) = frames.borrow_mut().last_mut()
            && let Some(reads) = frame.as_any().downcast_mut::<Vec<Read<D>>>()
        {
            reads.push(Read::Derived(read()));
        }
    });
}

/// Notes `reads`, which a kept answer or an inner recording read, for the recording now: its
/// table reads in a recording of any kind, its derived reads in one of its kind.
pub fn note_all<D: Clone + 'static>(kept: &[Read<D>]) {
    FRAMES.with(|frames| {
        let mut frames = frames.borrow_mut();
        let Some(frame) = frames.last_mut() else {
            return;
        };
        if let Some(reads) = frame.as_any().downcast_mut::<Vec<Read<D>>>() {
            reads.extend_from_slice(kept);
            return;
        }
        for read in kept {
            match *read {
                Read::Row { table, key, fields } => {
                    frame.table_read(TableRead::Row(table, key, fields));
                }
                Read::Range { table, from, to, fields } => {
                    frame.table_read(TableRead::Range(table, from, to, fields));
                }
                Read::Table { table, fields } => frame.table_read(TableRead::Table(table, fields)),
                Read::Derived(_) => {}
            }
        }
    });
}

/// Runs `work` on `frame`, returning what it read, each once: what it noted, and each table
/// of `frame` it read without noting, whole. Recordings nest: one around this one also holds
/// what was read inside.
pub fn record<'f, D: Clone + Ord + 'static, T>(
    frame: impl Into<HourFrame<'f>>,
    work: impl FnOnce() -> T,
) -> (T, Vec<Read<D>>) {
    // Pops this recording even when `work` unwinds.
    struct Pop(bool);
    impl Drop for Pop {
        fn drop(&mut self) {
            if self.0 {
                FRAMES.with(|frames| frames.borrow_mut().pop());
            }
        }
    }
    let frame = frame.into();
    FRAMES.with(|frames| frames.borrow_mut().push(Box::new(Vec::<Read<D>>::new())));
    let mut pop = Pop(true);
    let (value, views, walked) = view_reads::record_walked(work);
    pop.0 = false;
    let mut reads = FRAMES.with(|frames| {
        let mut frames = frames.borrow_mut();
        let mut top = frames.pop().expect("this recording");
        std::mem::take(top.as_any().downcast_mut::<Vec<Read<D>>>().expect("this recording's kind"))
    });
    let noted: HashSet<TableId> = reads.iter().filter_map(Read::table).collect();
    let decoded = frame
        .decoded()
        .into_iter()
        .flat_map(|decoded| decoded.decoded_frame().tables.iter().zip(TableId::ALL))
        .filter(|(view, _)| views.iter().any(|read| read.is(view)))
        .map(|(_, table)| table);
    let whole: Vec<_> = decoded
        .chain(walked)
        .filter(|table| !noted.contains(table))
        .map(|table| Read::Table { table, fields: u64::MAX })
        .collect();
    reads.extend(whole);
    reads.sort_unstable();
    reads.dedup();
    note_all(&reads);
    (value, reads)
}

/// The rows `now` changes from `then` in `table`, ascending by key, each with the fields it
/// changed (every field for one `now` inserts), and the keys it deletes; None when either
/// view is not complete.
pub fn changed_rows(
    table: TableId,
    then: &SharedView,
    now: &SharedView,
) -> Option<(Vec<(u128, u64)>, Vec<u128>)> {
    if SharedView::ptr_eq(then, now) {
        return Some((Vec::new(), Vec::new()));
    }
    match (&**then, &**now) {
        (TableView::Complete(old), TableView::Complete(new)) => match old.visit_pair(new, Diff) {
            Ok(rows) => Some(rows),
            Err(_) => {
                mismatched(table);
                None
            }
        },
        _ => None,
    }
}

/// Two views of `table` that hold different kinds of rows, a defect: counted, logged and
/// failed in debug builds. Answers that read it are made again.
fn mismatched(table: TableId) {
    static MISMATCHED: AtomicU64 = AtomicU64::new(0);
    let count = MISMATCHED.fetch_add(1, Ordering::Relaxed) + 1;
    if count.is_power_of_two() {
        eprintln!("kept: two views of {table:?} hold different kinds of rows ({count} so far)");
    }
    debug_assert!(false, "kept: two views of {table:?} hold different kinds of rows");
}

struct Diff;

impl RowsPairVisit for Diff {
    type Output = (Vec<(u128, u64)>, Vec<u128>);

    fn rows<R: RowCodec>(self, old: &[R], new: &[R]) -> Self::Output {
        let (mut changed, mut deleted) = (Vec::new(), Vec::new());
        let (mut a, mut b) = (0, 0);
        loop {
            match (old.get(a), new.get(b)) {
                (Some(was), Some(is)) if was.key() == is.key() => {
                    if was != is {
                        let fields = was.differing_fields(is);
                        if fields != 0 {
                            changed.push((is.key(), fields));
                        }
                    }
                    a += 1;
                    b += 1;
                }
                (Some(was), Some(is)) if was.key() < is.key() => {
                    deleted.push(was.key());
                    a += 1;
                }
                (Some(was), None) => {
                    deleted.push(was.key());
                    a += 1;
                }
                (_, Some(is)) => {
                    changed.push((is.key(), u64::MAX));
                    b += 1;
                }
                (None, None) => break,
            }
        }
        (changed, deleted)
    }
}

/// Two tables' rows keyed in `from..to` compared in some fields.
struct Compare {
    from: u128,
    to: u128,
    fields: u64,
}

impl RowsPairVisit for Compare {
    type Output = bool;

    fn rows<R: RowCodec>(self, old: &[R], new: &[R]) -> bool {
        fn range<R: RowCodec>(rows: &[R], from: u128, to: u128) -> &[R] {
            let start = rows.partition_point(|row| row.key() < from);
            let end = rows.partition_point(|row| row.key() < to);
            &rows[start..end.max(start)]
        }
        let (old, new) = (range(old, self.from, self.to), range(new, self.from, self.to));
        old.len() == new.len()
            && old.iter().zip(new).all(|(old, new)| {
                old.key() == new.key()
                    && (old == new || old.differing_fields(new) & self.fields == 0)
            })
    }
}

/// A kept answer and what it read.
pub struct Answer<V, D> {
    pub value: Arc<V>,
    pub reads: Arc<[Read<D>]>,
}

impl<V, D> Clone for Answer<V, D> {
    fn clone(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
            reads: Arc::clone(&self.reads),
        }
    }
}

/// The answers of one product kept by key, each with what it read. The holder keeps the hour
/// they were kept at, and gives the next hour the changes since then.
pub struct Kept<K, V, D> {
    answers: HashMap<K, Answer<V, D>>,
    index: Option<Dependents<K, D>>,
}

impl<K: Clone, V, D: Clone> Clone for Kept<K, V, D> {
    fn clone(&self) -> Self {
        Self {
            answers: self.answers.clone(),
            index: self.index.clone(),
        }
    }
}

impl<K, V, D> Default for Kept<K, V, D> {
    fn default() -> Self {
        Self {
            answers: HashMap::new(),
            index: None,
        }
    }
}

impl<K: Clone + Eq + Hash, V, D: Clone + Eq + Hash + Grouped + 'static> Kept<K, V, D> {
    /// Answers checked when asked for: for an hour that asks for few of many kept.
    pub fn checked() -> Self {
        Self::default()
    }

    /// Answers indexed by what they read: an hour drops those its changes reach, and keeps
    /// the rest unasked.
    pub fn indexed() -> Self {
        Self {
            answers: HashMap::new(),
            index: Some(Dependents::default()),
        }
    }

    /// The answers for an hour `changes` lead to from the hour they were kept at. `changed`
    /// says whether a derived read reads otherwise now.
    pub fn hour<'c, C: Changes + ?Sized, F: Fn(&D) -> bool>(
        mut self,
        changes: &'c C,
        changed: F,
    ) -> Checked<'c, K, V, D, C, F> {
        if let Some(index) = &mut self.index {
            for key in index.reached(changes, &changed) {
                index.remove(&key);
                self.answers.remove(&key);
            }
        }
        Checked {
            kept: self,
            changes,
            changed,
            checked: RefCell::new(HashMap::new()),
            confirmed: HashSet::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.answers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.answers.is_empty()
    }
}

/// One hour's use of [`Kept`] answers: those still held, and those the hour makes, kept for
/// the next.
pub struct Checked<'c, K, V, D, C: ?Sized, F> {
    kept: Kept<K, V, D>,
    changes: &'c C,
    changed: F,
    /// A plain [`Kept`]'s reads checked this hour, and the answers confirmed or made.
    checked: RefCell<HashMap<Read<D>, bool>>,
    confirmed: HashSet<K>,
}

impl<K, V, D, C, F> Checked<'_, K, V, D, C, F>
where
    K: Clone + Eq + Hash,
    D: Clone + Eq + Hash + Grouped + 'static,
    C: Changes + ?Sized,
    F: Fn(&D) -> bool,
{
    /// Whether every one of `reads` reads the same now as at the kept hour.
    pub fn holds(&self, reads: &[Read<D>]) -> bool {
        let mut checked = self.checked.borrow_mut();
        reads.iter().all(|read| {
            if let Some(&same) = checked.get(read) {
                return same;
            }
            let same = match *read {
                Read::Row { table, key, fields } => {
                    self.rows_hold(table, key, key.saturating_add(1), fields)
                }
                Read::Range { table, from, to, fields } => self.rows_hold(table, from, to, fields),
                Read::Table { table, fields } => self.rows_hold(table, 0, u128::MAX, fields),
                Read::Derived(ref read) => {
                    read.table()
                        .is_some_and(|table| matches!(self.changes.table(table), Change::Same))
                        || !(self.changed)(read)
                }
            };
            checked.insert(read.clone(), same);
            same
        })
    }

    fn rows_hold(&self, table: TableId, from: u128, to: u128, fields: u64) -> bool {
        match self.changes.table(table) {
            Change::Same => true,
            Change::Fields(changed) if changed & fields == 0 => true,
            Change::Rows(changed, deleted) => !reaches(changed, deleted, from, to, fields),
            Change::Fields(_) | Change::Any => self
                .changes
                .same_rows(table, from, to, fields)
                .or_else(|| {
                    let (changed, deleted) = self.changes.rows(table)?;
                    Some(!reaches(changed, deleted, from, to, fields))
                })
                .unwrap_or(false),
        }
    }

    /// The answer kept for `key`, when it still holds; noted for any recording now.
    pub fn get(&mut self, key: &K) -> Option<Answer<V, D>> {
        let answer = self.kept.answers.get(key)?;
        if self.kept.index.is_none() && !self.confirmed.contains(key) {
            if !self.holds(&answer.reads) {
                self.kept.answers.remove(key);
                return None;
            }
            self.confirmed.insert(key.clone());
        }
        let answer = self.kept.answers[key].clone();
        note_all(&answer.reads);
        Some(answer)
    }

    /// `key`'s answer: kept when it holds, else `build`'s on `frame`, recorded.
    pub fn get_or_build<'f>(
        &mut self,
        key: K,
        frame: impl Into<HourFrame<'f>>,
        build: impl FnOnce() -> V,
    ) -> Arc<V>
    where
        D: Ord,
    {
        if let Some(answer) = self.get(&key) {
            return answer.value;
        }
        let (value, reads) = record(frame, build);
        self.put(key, value, reads).value
    }

    /// Keeps `value` for `key` with the reads that made it.
    pub fn put(&mut self, key: K, value: V, reads: Vec<Read<D>>) -> Answer<V, D> {
        let answer = Answer {
            value: Arc::new(value),
            reads: reads.into(),
        };
        match &mut self.kept.index {
            Some(index) => index.set(key.clone(), Arc::clone(&answer.reads)),
            None => {
                self.confirmed.insert(key.clone());
            }
        }
        self.kept.answers.insert(key, answer.clone());
        answer
    }

    /// The answers for the next hour: every one that still holds.
    pub fn finish(mut self) -> Kept<K, V, D> {
        if self.kept.index.is_none() {
            let mut answers = std::mem::take(&mut self.kept.answers);
            answers.retain(|key, answer| self.confirmed.contains(key) || self.holds(&answer.reads));
            self.kept.answers = answers;
        }
        self.kept
    }
}
