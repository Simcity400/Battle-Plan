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
//! [`replay::frame::view_reads`].

use std::{
    cell::{Cell, RefCell},
    hash::{Hash, Hasher},
};

use replay::{
    frame::{DecodedFrame, ViewIdentity, view_reads},
    rows::TableId,
};

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

/// The value an input read: a digest, or a table view's identity, which
/// keeps the view's allocation reserved so no later view can share it.
#[derive(Clone, Debug)]
enum Value {
    Digest(u64),
    View(ViewIdentity),
}

/// The reads of one evaluation, each key once, in first-read order.
#[derive(Clone, Debug, Default)]
pub struct Inputs {
    reads: Vec<(Key, Value)>,
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
    let inputs = RECORDING.with(|recording| {
        let mut recording = recording.borrow_mut();
        let inputs = recording.last_mut().map(std::mem::take).unwrap_or_default();
        // An enclosing evaluation read everything this one read.
        if let Some(outer) = recording.iter_mut().rev().nth(1) {
            for (key, value) in &inputs.reads {
                outer.note(key, value);
            }
        }
        inputs
    });
    drop(pop);
    (value, inputs)
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
pub fn table(frame: &DecodedFrame, table: TableId) {
    if recording() {
        record(Key::Table(table), || {
            Value::View(frame.tables[table.index()].identity())
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

impl Inputs {
    fn note(&mut self, key: &Key, value: &Value) {
        // The first read decided the evaluation: a key reads one value.
        if !self.reads.iter().any(|(read, _)| read == key) {
            self.reads.push((key.clone(), value.clone()));
        }
    }

    /// Whether every input reads its recorded value again: a table is still
    /// the same view of `frame`, and `reread` gives every other key's digest
    /// now, None when it cannot read it.
    pub fn confirm(
        &self,
        frame: &DecodedFrame,
        mut reread: impl FnMut(&Key) -> Option<u64>,
    ) -> bool {
        self.reads.iter().all(|(key, value)| match (key, value) {
            (Key::Table(table), Value::View(view)) => view.is(&frame.tables[table.index()]),
            (_, Value::Digest(digest)) => reread(key) == Some(*digest),
            _ => false,
        })
    }

    /// The tables read whole.
    #[cfg(test)]
    fn tables(&self) -> impl Iterator<Item = TableId> + '_ {
        self.reads.iter().filter_map(|(key, _)| match key {
            Key::Table(table) => Some(*table),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::frame::TableView;
    use replay::rows::{TableRows, tables};

    #[test]
    fn evaluations_record_queries_and_whole_tables_read_outside_them() {
        let frame = crate::test_fixtures::source(1, 10, Some(7));
        let countries = |frame: &DecodedFrame| frame.complete::<tables::Countries>().unwrap().len();
        let (inner, outer) = capture(&frame, || {
            // Read inside a query: only the query's result is an input.
            let rows = query(|| Key::Table(TableId::States), || countries(&frame));
            assert_eq!(rows, 1);
            let ((), inner) = capture(&frame, || {
                countries(&frame);
            });
            inner
        });
        assert_eq!(inner.tables().collect::<Vec<_>>(), [TableId::Countries]);
        assert_eq!(
            outer.reads.len(),
            2,
            "the query, and the inner evaluation's table"
        );
        let unchanged = |_: &Key| Some(digest(&1_usize));
        assert!(outer.confirm(&frame, unchanged));
        assert!(!outer.confirm(&frame, |_| Some(digest(&2_usize))));
        let mut changed = frame.clone();
        *changed.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::empty(TableId::Countries));
        assert!(!outer.confirm(&changed, unchanged));
        // Shared builds and reads outside an evaluation record nothing.
        let ((), shared) = capture(&frame, || unrecorded(|| assert_eq!(countries(&frame), 1)));
        assert_eq!(shared.reads.len(), 0);
    }
}
