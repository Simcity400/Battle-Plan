//! The view a frame of rows made at the playback head holds for a table its reader did not
//! declare. It is never "not recorded" and never empty: reading it panics in a debug build and,
//! in a release build, is counted and reads as unavailable with [`NOT_DECLARED`] in the reason.
//! A reader that learns its tables by reading (a script evaluator, a tooltip over hundreds of
//! leaf functions) reads inside [`discovering`], which names the undeclared tables it read.
//!
//! A table declared for some of its columns is held in a view that knows them
//! ([`SharedView::with_fields`]). A read names the fields it reads ([`reading`]; a read that names
//! none reads every field); one outside the columns held is refused the same way and, inside
//! [`discovering_fields`], names the table with the fields it asked for.

use super::{SharedView, TableView};
use crate::rows::TableId;
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

pub const NOT_DECLARED: &str = "Table not declared by its reader of rows at the head";

/// One view per table, and their addresses ascending with the table each is.
struct Views {
    views: Vec<Arc<TableView>>,
    sorted: Vec<(usize, TableId)>,
}

static VIEWS: OnceLock<Views> = OnceLock::new();
static LOW: AtomicUsize = AtomicUsize::new(usize::MAX);
static HIGH: AtomicUsize = AtomicUsize::new(0);
static READS: AtomicU64 = AtomicU64::new(0);
static OUTSIDE: AtomicU64 = AtomicU64::new(0);

thread_local! {
    static DISCOVERING: RefCell<Vec<Vec<(TableId, u64)>>> = const { RefCell::new(Vec::new()) };
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    /// The fields the read under way names: every field unless a masked read set them.
    static READING: Cell<u64> = const { Cell::new(u64::MAX) };
    /// Inside a discovery: the rows of each view holding only some columns that a masked read
    /// gave, as an address range, with its table and columns, so a column read later from those
    /// rows (a text, a list) is checked too.
    static HELD_ROWS: RefCell<Vec<(usize, usize, TableId, u64)>> = const { RefCell::new(Vec::new()) };
}

fn views() -> &'static Views {
    VIEWS.get_or_init(|| {
        let views: Vec<Arc<TableView>> = TableId::ALL.iter().map(|table| Arc::new(TableView::Unavailable { reason: format!("{table:?}: {NOT_DECLARED}") })).collect();
        let mut sorted: Vec<(usize, TableId)> = views.iter().zip(TableId::ALL).map(|(view, table)| (Arc::as_ptr(view) as usize, table)).collect();
        sorted.sort_unstable_by_key(|(address, _)| *address);
        HIGH.store(sorted.last().map_or(0, |(address, _)| *address), Ordering::Release);
        LOW.store(sorted.first().map_or(usize::MAX, |(address, _)| *address), Ordering::Release);
        Views { views, sorted }
    })
}

/// The view `table` holds in every frame at the head whose reader did not declare it.
pub fn not_declared(table: TableId) -> SharedView {
    SharedView(Arc::clone(&views().views[table.index()]), None, None, None, None)
}

/// Reads of an undeclared table since the process started.
pub fn undeclared_reads() -> u64 {
    READS.load(Ordering::Relaxed)
}

/// Reads of fields outside the columns a view at the head holds since the process started.
pub fn outside_reads() -> u64 {
    OUTSIDE.load(Ordering::Relaxed)
}

/// `read`, which reads the fields of `fields` of every table it reads.
pub fn reading<T>(fields: u64, read: impl FnOnce() -> T) -> T {
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            READING.with(|reading| reading.set(self.0));
        }
    }
    let _restore = Restore(READING.with(|reading| reading.replace(fields)));
    read()
}

/// `read`, and the undeclared tables it read, each once: those reads are counted but not refused.
pub fn discovering<T>(read: impl FnOnce() -> T) -> (T, Vec<TableId>) {
    let (value, found) = discovering_fields(read);
    (value, found.into_iter().map(|(table, _)| table).collect())
}

/// [`discovering`] with the fields each table was read for: the union of every read's, every
/// field for a read that named none.
pub fn discovering_fields<T>(read: impl FnOnce() -> T) -> (T, Vec<(TableId, u64)>) {
    struct Leave;
    impl Drop for Leave {
        fn drop(&mut self) {
            let depth = DEPTH.with(|depth| {
                depth.set(depth.get() - 1);
                depth.get()
            });
            DISCOVERING.with(|found| found.borrow_mut().pop());
            if depth == 0 {
                HELD_ROWS.with(|held| held.borrow_mut().clear());
            }
        }
    }
    DISCOVERING.with(|found| found.borrow_mut().push(Vec::new()));
    DEPTH.with(|depth| depth.set(depth.get() + 1));
    let leave = Leave;
    let value = read();
    let found = DISCOVERING.with(|found| found.borrow_mut().last_mut().map(std::mem::take).unwrap_or_default());
    drop(leave);
    (value, found)
}

#[inline]
fn address(view: &Arc<TableView>) -> usize {
    Arc::as_ptr(view) as usize
}

#[inline]
pub(super) fn is(view: &Arc<TableView>) -> bool {
    let at = address(view);
    at >= LOW.load(Ordering::Relaxed) && at <= HIGH.load(Ordering::Relaxed) && table(at).is_some()
}

#[cold]
fn table(at: usize) -> Option<TableId> {
    let sorted = &VIEWS.get()?.sorted;
    sorted.binary_search_by_key(&at, |(address, _)| *address).ok().map(|found| sorted[found].1)
}

/// Notes `table` read for `fields` in every discovery under way on this thread.
fn note(table: TableId, fields: u64) {
    DISCOVERING.with(|found| {
        for found in found.borrow_mut().iter_mut() {
            match found.iter_mut().find(|(held, _)| *held == table) {
                Some((_, held)) => *held |= fields,
                None => found.push((table, fields)),
            }
        }
    });
}

#[cold]
pub(super) fn read(view: &Arc<TableView>) {
    READS.fetch_add(1, Ordering::Relaxed);
    let Some(table) = table(address(view)) else { return };
    if DEPTH.with(Cell::get) > 0 {
        note(table, READING.with(Cell::get));
    } else if cfg!(debug_assertions) {
        panic!("a reader of rows at the head read {table:?}, which it did not declare");
    }
}

/// Inside a discovery, keeps the rows `rows` (an address range) of `table`'s view holding the
/// columns of `held`, for [`column_read`].
pub(super) fn held_rows(rows: std::ops::Range<usize>, table: TableId, held: u64) {
    if DEPTH.with(Cell::get) == 0 {
        return;
    }
    HELD_ROWS.with(|kept| {
        let mut kept = kept.borrow_mut();
        if !kept.iter().any(|&(start, end, _, _)| (start, end) == (rows.start, rows.end)) {
            kept.push((rows.start, rows.end, table, held));
        }
    });
}

/// Inside a discovery, a read of `column` from rows starting at `at`: when they are rows of a view
/// holding other columns, the column is noted for the table, so the need widens.
pub fn column_read(at: usize, column: usize) {
    if DEPTH.with(Cell::get) == 0 || column >= 64 {
        return;
    }
    let found = HELD_ROWS.with(|kept| kept.borrow().iter().find(|&&(start, end, _, _)| start <= at && at <= end).map(|&(_, _, table, held)| (table, held)));
    if let Some((table, held)) = found
        && held & (1 << column) == 0
    {
        OUTSIDE.fetch_add(1, Ordering::Relaxed);
        note(table, 1 << column);
    }
}

/// The view a read of `table` sees when its view holds only the columns of `held`: none when the
/// read names fields among them; the table's undeclared view when it reads others, which are
/// never served (noted for discovery, a panic outside it in a debug build, counted).
#[cold]
pub(super) fn outside(table: TableId, held: u64) -> Option<&'static TableView> {
    let asked = READING.with(Cell::get);
    let missing = asked & !held;
    if missing == 0 {
        return None;
    }
    OUTSIDE.fetch_add(1, Ordering::Relaxed);
    if DEPTH.with(Cell::get) > 0 {
        note(table, asked);
    } else if cfg!(debug_assertions) {
        let names: Vec<&str> = (0..64).filter(|bit| missing & (1 << bit) != 0).filter_map(|bit| table.field_name(bit)).collect();
        panic!("a reader of rows at the head read {table:?}'s {names:?}, outside the columns it declared");
    }
    Some(&views().views[table.index()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_an_undeclared_table_is_a_loud_failure_and_discovery_names_it() {
        let view = not_declared(TableId::Countries);
        assert!(view.is_not_declared());
        assert!(!SharedView::from(TableView::Unavailable { reason: NOT_DECLARED.into() }).is_not_declared());
        let before = undeclared_reads();
        let (reason, found) = discovering(|| {
            let _ = &*not_declared(TableId::Provinces);
            match &*view {
                TableView::Unavailable { reason } => reason.clone(),
                _ => String::new(),
            }
        });
        assert!(reason.contains(NOT_DECLARED) && reason.starts_with("Countries"), "{reason}");
        assert_eq!(found, vec![TableId::Provinces, TableId::Countries]);
        // Counted process-wide: another test may read an undeclared view at the same moment.
        assert!(undeclared_reads() >= before + 2);
        let read = std::panic::catch_unwind(|| matches!(&*not_declared(TableId::States), TableView::Unavailable { .. }));
        assert_eq!(read.is_err(), cfg!(debug_assertions));
    }

    /// A leaf that asks when a table was sampled before it reads the rows (the naval dominance
    /// hover's NavalSupremacy) has read that table.
    #[test]
    fn asking_an_undeclared_tables_sample_hour_is_a_read_that_discovery_names() {
        let before = undeclared_reads();
        let (sampled, found) = discovering(|| not_declared(TableId::NavalSupremacy).sampled_at());
        assert_eq!(sampled, None);
        assert_eq!(found, vec![TableId::NavalSupremacy]);
        // Counted process-wide: another test may read an undeclared view at the same moment.
        assert!(undeclared_reads() > before);
    }

    /// A masked read names fields; a text or list column read afterwards from the same rows (as
    /// `has_tech` reads a technology's name with `with_text`) is a read of that column too: inside
    /// a discovery it is noted for the table, so the need widens instead of serving the column's
    /// zero.
    #[test]
    fn a_column_read_from_masked_rows_of_a_view_holding_other_columns_is_noted() {
        use crate::frame::{DecodedFrame, HourFrame, Metrics};
        use crate::rows::{CountryRow, TableRows, mask, tables};
        let stability = mask::<CountryRow>(&["stability_raw"]);
        let tag = crate::rows::column::<CountryRow>("tag");
        let mut frame = DecodedFrame { branch: 1, hour: 5, tables: std::array::from_fn(|at| SharedView::from(TableView::Complete(TableRows::empty(TableId::ALL[at])))), metrics: Metrics::default() };
        let rows = TableRows::Countries(vec![CountryRow { id: 7, tag: b"GER".to_vec(), stability_raw: 3, ..Default::default() }]);
        frame.tables[TableId::Countries.index()] = SharedView::from(TableView::Complete(rows)).with_fields(stability);
        let (stabilities, found) = discovering_fields(|| {
            let rows = HourFrame::Decoded(&frame).complete::<tables::Countries>(stability).unwrap();
            let _ = rows.text(7, |row| &row.tag, tag);
            rows.iter().map(|row| row.map(|row| row.stability_raw)).collect::<Result<Vec<_>, _>>().unwrap()
        });
        assert_eq!(stabilities, vec![3], "the fields named are served");
        assert_eq!(found, vec![(TableId::Countries, 1 << tag)], "the column read afterwards is named");
    }
}
