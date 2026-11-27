
use super::{Projection, SharedView, TableView};
use crate::rows::TableId;
use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

pub const NOT_DECLARED: &str = "Table not declared by its reader of rows at the head";

struct Views {
    views: Vec<Arc<TableView>>,
    sorted: Vec<(usize, TableId)>,
}

static VIEWS: OnceLock<Views> = OnceLock::new();
static LOW: AtomicUsize = AtomicUsize::new(usize::MAX);
static HIGH: AtomicUsize = AtomicUsize::new(0);
static READS: AtomicU64 = AtomicU64::new(0);
static OUTSIDE: AtomicU64 = AtomicU64::new(0);

type Projections = Vec<(TableId, Option<u64>)>;

thread_local! {
    static DISCOVERING: RefCell<Vec<Projections>> = const { RefCell::new(Vec::new()) };
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    static READING: Cell<Option<u64>> = const { Cell::new(Some(u64::MAX)) };
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

pub fn not_declared(table: TableId) -> SharedView {
    SharedView(Arc::clone(&views().views[table.index()]), None, None, None, None, None)
}

pub fn undeclared_reads() -> u64 {
    READS.load(Ordering::Relaxed)
}

pub fn outside_reads() -> u64 {
    OUTSIDE.load(Ordering::Relaxed)
}

pub fn reading<T>(fields: u64, read: impl FnOnce() -> T) -> T {
    access(Some(fields), read)
}

pub(super) fn status<T>(read: impl FnOnce() -> T) -> T {
    access(None, read)
}

fn access<T>(fields: Option<u64>, read: impl FnOnce() -> T) -> T {
    struct Restore(Option<u64>);
    impl Drop for Restore {
        fn drop(&mut self) {
            READING.with(|reading| reading.set(self.0));
        }
    }
    let _restore = Restore(READING.with(|reading| reading.replace(fields)));
    read()
}

pub fn discovering<T>(read: impl FnOnce() -> T) -> (T, Vec<TableId>) {
    let (value, found) = discovering_fields(read);
    (value, found.into_iter().map(|(table, _)| table).collect())
}

pub fn discovering_within<T>(read: impl FnOnce() -> T) -> (T, Vec<TableId>) {
    if DEPTH.with(Cell::get) == 0 {
        return (read(), Vec::new());
    }
    discovering(read)
}

pub fn discovering_fields<T>(read: impl FnOnce() -> T) -> (T, Vec<(TableId, u64)>) {
    let (value, found) = discovering_projections(read);
    (value, found.into_iter().map(|(table, fields)| (table, fields.unwrap_or(0))).collect())
}

pub fn discovering_projections<T>(read: impl FnOnce() -> T) -> (T, Projections) {
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

pub struct Discovery(bool);

impl Discovery {
    pub fn current() -> Self {
        Self(DEPTH.with(Cell::get) > 0)
    }

    pub fn run<T>(self, read: impl FnOnce() -> T) -> (T, Projections) {
        if self.0 { discovering_projections(read) } else { (read(), Vec::new()) }
    }

    pub fn credit(found: Projections) {
        for (table, fields) in found { note(table, fields); }
    }
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

fn note(table: TableId, fields: Option<u64>) {
    DISCOVERING.with(|found| {
        for found in found.borrow_mut().iter_mut() {
            match found.iter_mut().find(|(held, _)| *held == table) {
                Some((_, held)) => {
                    if let Some(fields) = fields { *held = Some(held.unwrap_or(0) | fields); }
                }
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

pub fn column_read(at: usize, column: usize) {
    if DEPTH.with(Cell::get) == 0 || column >= 64 {
        return;
    }
    let found = HELD_ROWS.with(|kept| kept.borrow().iter().find(|&&(start, end, _, _)| start <= at && at <= end).map(|&(_, _, table, held)| (table, held)));
    if let Some((table, held)) = found
        && held & (1 << column) == 0
    {
        OUTSIDE.fetch_add(1, Ordering::Relaxed);
        note(table, Some(1 << column));
    }
}

#[cold]
pub(super) fn outside(table: TableId, held: Projection) -> Option<&'static TableView> {
    let asked = READING.with(Cell::get)? & table.full_mask();
    let missing = match held {
        Projection::Status => asked,
        Projection::Fields(held) => asked & !held,
    };
    if missing == 0 && matches!(held, Projection::Fields(_)) {
        return None;
    }
    OUTSIDE.fetch_add(1, Ordering::Relaxed);
    if DEPTH.with(Cell::get) > 0 {
        note(table, Some(asked));
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
        assert!(undeclared_reads() >= before + 2);
        let read = std::panic::catch_unwind(|| matches!(&*not_declared(TableId::States), TableView::Unavailable { .. }));
        assert_eq!(read.is_err(), cfg!(debug_assertions));
    }

    #[test]
    fn asking_an_undeclared_tables_sample_hour_is_a_read_that_discovery_names() {
        let before = undeclared_reads();
        let (sampled, found) = discovering(|| not_declared(TableId::NavalSupremacy).sampled_at());
        assert_eq!(sampled, None);
        assert_eq!(found, vec![TableId::NavalSupremacy]);
        assert!(undeclared_reads() > before);
    }

    #[test]
    fn a_status_projection_can_be_read_without_rows_and_upgraded_on_a_worker() {
        use crate::frame::OperandStatus;
        use crate::rows::TableRows;

        let table = TableId::States;
        let missing = not_declared(table);
        let (_, requested) = discovering_projections(|| status(|| OperandStatus::for_table(&missing, table)));
        assert_eq!(requested, [(table, None)]);

        let held = SharedView::from(TableView::Complete(TableRows::empty(table))).status_only();
        let ((state, membership), requested) = discovering_projections(|| {
            let state = status(|| OperandStatus::for_table(&held, table));
            let discovery = Discovery::current();
            let (membership, requested) = std::thread::scope(|worker| {
                worker.spawn(|| discovery.run(|| reading(0, || OperandStatus::for_table(&held, table)))).join().unwrap()
            });
            Discovery::credit(requested);
            (state, membership)
        });
        assert_eq!(state, OperandStatus::Complete);
        assert!(matches!(membership, OperandStatus::Unavailable(_)));
        assert_eq!(requested, [(table, Some(0))]);
    }

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
