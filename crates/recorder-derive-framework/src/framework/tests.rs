use super::*;
use crate::testing::schedule::{EVENTS_TABLES, LOG, Memory, ORDERED, TOY, TOY_KINDS, apply, reason, rows_of, schema, store};
use replay_data::rows::TABLE_COUNT;
use std::sync::Arc;

struct Setting {
    rows: Vec<(u128, Value)>,
}

impl Writer for Setting {
    fn name(&self) -> &'static str {
        "Setting"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::Campaign)]
    }

    fn reads(&self) -> &'static [Read] {
        &[]
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {}

    fn hour(&mut self, _: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        for &(key, cell) in &self.rows {
            out.set(TableId::Campaign, key, vec![cell]);
        }
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_writer_setting_equal_rows_writes_nothing() {
    let mut store = store();
    store.begin_hour(0, &[b"a", b"b"]).unwrap();
    let mut writers = Writers::new(vec![Box::new(Setting { rows: vec![(1, Value::Bytes(0)), (2, Value::Bytes(1))] })]);
    writers.start_run();
    writers.hour(1, 0, true, &mut store).unwrap();
    let campaign = TableId::Campaign.index();
    assert_eq!(store.log().tables[campaign].inserted.len(), 2);
    store.begin_hour(1, &[]).unwrap();
    writers.hour(1, 1, false, &mut store).unwrap();
    let log = &store.log().tables[campaign];
    assert!(log.inserted.is_empty() && log.deleted.is_empty() && log.written.is_empty(), "{log:?}");
    let mut writers = Writers::new(vec![Box::new(Setting { rows: vec![(1, Value::Bytes(1)), (2, Value::Unrecorded)] })]);
    store.begin_hour(2, &[]).unwrap();
    writers.hour(1, 2, false, &mut store).unwrap();
    let log = &store.log().tables[campaign];
    assert_eq!(log.written.len(), 2);
    let table = store.view(campaign);
    let slot = table.slot(2).unwrap();
    assert!(!table.recorded(0, slot));
    assert_eq!(table.text(0, table.slot(1).unwrap()), Some(1));
}

#[cfg(feature = "memory-labels")]
mod labelled {
    use super::*;

    struct Hoarding {
        bytes: usize,
        rows: u32,
        held: Vec<Vec<u8>>,
    }

    impl Writer for Hoarding {
        fn name(&self) -> &'static str {
            "Hoarding"
        }

        fn version(&self) -> u32 {
            1
        }

        fn tables(&self) -> &'static [Output] {
            &[Output::State(TableId::Campaign)]
        }

        fn reads(&self) -> &'static [Read] {
            &[]
        }

        fn start(&mut self) {}

        fn hour(&mut self, _: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
            self.held.push(vec![1; self.bytes]);
            for key in 0..self.rows {
                out.set(TableId::Campaign, u128::from(key), vec![Value::Bytes(0)]);
            }
            Ok(())
        }

        fn save(&self, _: &mut Bytes) -> io::Result<()> {
            Ok(())
        }

        fn load(&mut self, _: &[u8]) -> io::Result<()> {
            Ok(())
        }
    }

}

#[test]
fn a_table_the_hour_landed_and_a_writer_writes_is_refused_by_name() {
    let mut store = store();
    let mut writers = Writers::new(vec![Box::new(Setting { rows: vec![(1, Value::Bytes(0))] })]);
    writers.start_run();
    store.begin_hour(0, &[b"a"]).unwrap();
    writers.hour(1, 0, true, &mut store).unwrap();
    store.begin_hour(1, &[]).unwrap();
    writers.hour(1, 1, false, &mut store).unwrap();
    store.begin_hour(2, &[]).unwrap();
    store.status(TableId::Campaign.index(), Status::Unavailable { reason: 0 }, true, Some(2), false).unwrap();
    let refused = writers.hour(1, 2, false, &mut store).unwrap_err().to_string();
    assert!(refused.contains("Campaign is Setting's, and the hour landed it before Setting ran"), "{refused}");
}

const EVENTS_OUTPUTS: [Output; 2] = [Output::State(EVENTS_TABLES[0]), Output::State(EVENTS_TABLES[1])];
const EVENTS: &[Read] = &[Read { table: LOG, columns: Columns::Every, watch: Watch::Appended, required: false }];

fn log_store() -> Store {
    let mut store = store();
    let mut logs = vec![false; TABLE_COUNT];
    logs[LOG.index()] = true;
    store.declare_logs(&logs).unwrap();
    store
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Handed {
    hour: i32,
    appended: Vec<(u64, u128, u64)>,
    backlog: bool,
    forgotten: Vec<std::ops::Range<u64>>,
}

type Handings = std::sync::Arc<std::sync::Mutex<Vec<Handed>>>;

struct Follow {
    name: &'static str,
    tables: &'static [Output; 2],
    reads: &'static [Read],
    handed: Handings,
}

fn rows_of_log(rows: LogRows<'_>) -> Vec<(u64, u128, u64)> {
    rows.rows().map(|row| (row.ordinal, row.key, rows.cells().and_then(|cells| cells.number(0, row.slot)).unwrap_or(-1) as u64)).collect()
}

impl Writer for Follow {
    fn name(&self) -> &'static str {
        self.name
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        self.tables
    }

    fn reads(&self) -> &'static [Read] {
        self.reads
    }

    fn start(&mut self) {}

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let [by_key, by_ordinal] = self.tables.map(Output::table);
        let appended = hour.appended(LOG);
        let handed = Handed {
            hour: hour.hour,
            appended: rows_of_log(appended),
            backlog: appended.backlog(),
            forgotten: hour.forgotten(LOG).to_vec(),
        };
        let mut keys: std::collections::BTreeMap<u128, (u64, u64)> = std::collections::BTreeMap::new();
        let mut ordinals: std::collections::BTreeMap<u64, u128> = std::collections::BTreeMap::new();
        for &(ordinal, key, value) in &handed.appended {
            keys.insert(key, (ordinal, value));
            ordinals.insert(ordinal, key);
        }
        let mut deletes = Vec::new();
        {
            let (held_keys, held_ordinals) = (out.table(by_key), out.table(by_ordinal));
            for ordinal in handed.forgotten.iter().flat_map(Clone::clone) {
                let key = ordinals.remove(&ordinal).or_else(|| held_ordinals.slot(u128::from(ordinal)).and_then(|slot| held_ordinals.number(0, slot)).map(|key| key as u128));
                deletes.push((by_ordinal, u128::from(ordinal)));
                let Some(key) = key else { continue };
                let newest = keys.get(&key).map(|(at, _)| *at).or_else(|| held_keys.slot(key).and_then(|slot| held_keys.number(0, slot)).map(|at| at as u64));
                if newest == Some(ordinal) {
                    keys.remove(&key);
                    deletes.push((by_key, key));
                }
            }
        }
        for (table, key) in deletes {
            out.delete(table, key);
        }
        for (key, (ordinal, value)) in keys {
            out.set(by_key, key, vec![Value::U64(ordinal), Value::U64(value)]);
        }
        for (ordinal, key) in ordinals {
            out.set(by_ordinal, u128::from(ordinal), vec![Value::U64(key as u64)]);
        }
        self.handed.lock().unwrap().push(handed);
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

fn log_hour(store: &mut Store, hour: i32, appends: &[(u128, u64)], forgets: &[u64], status: Status) {
    store.begin_hour(hour, &[]).unwrap();
    for &(key, value) in appends {
        store.table(LOG.index()).unwrap().append(key, &[Some(Cell::U64(value)), Some(Cell::I32(hour))]).unwrap();
    }
    for &ordinal in forgets {
        store.table(LOG.index()).unwrap().forget(ordinal).unwrap();
    }
    store.status(LOG.index(), status, true, Some(hour), true).unwrap();
    store.settle();
}

fn keys_of(store: &Store, table: TableId) -> Vec<u128> {
    let view = store.view(table.index());
    let mut keys: Vec<u128> = view.rows().filter_map(|slot| view.key(slot)).collect();
    keys.sort_unstable();
    keys
}

fn follow(name: &'static str, tables: &'static [Output; 2], reads: &'static [Read], handed: &Handings) -> Box<dyn Writer> {
    Box::new(Follow { name, tables, reads, handed: handed.clone() })
}

#[test]
fn a_writer_reading_a_log_is_handed_its_appended_and_forgotten_rows() {
    let events = Handings::default();
    let mut store = log_store();
    let mut writers = Writers::new(vec![follow("Events", &EVENTS_OUTPUTS, EVENTS, &events)]);
    writers.start_run();
    store.begin_run(0);
    let end = |store: &mut Store| store.end_hour(&mut replay_columnar::unit::Discard).unwrap();
    let statuses = |store: &Store| EVENTS_TABLES.iter().map(|table| store.view(table.index()).status()).collect::<Vec<_>>();
    log_hour(&mut store, 0, &[(1, 10), (2, 20), (3, 30)], &[], Status::Complete);
    writers.hour(1, 0, true, &mut store).unwrap();
    end(&mut store);
    log_hour(&mut store, 1, &[(4, 40)], &[0], Status::Complete);
    writers.hour(1, 1, false, &mut store).unwrap();
    end(&mut store);
    assert_eq!((keys_of(&store, EVENTS_TABLES[0]), keys_of(&store, EVENTS_TABLES[1])), (vec![2, 3, 4], vec![1, 2, 3]), "the forgotten row gone by key and by ordinal");
    log_hour(&mut store, 2, &[], &[], Status::Complete);
    writers.hour(1, 2, false, &mut store).unwrap();
    assert!(events.lock().unwrap().iter().all(|handed| handed.hour != 2), "a reader of a log is not run in an hour that changed neither its rows nor its status");
    end(&mut store);
    log_hour(&mut store, 3, &[(5, 50)], &[], Status::Truncated { missing: 4 });
    writers.hour(1, 3, false, &mut store).unwrap();
    assert_eq!(statuses(&store), [Status::Truncated { missing: 4 }; 2], "a log read in part makes its readers' tables so");
    end(&mut store);
    let lost = store.intern(b"the capture lost the interval").unwrap();
    log_hour(&mut store, 4, &[], &[], Status::Unavailable { reason: lost });
    writers.hour(1, 4, false, &mut store).unwrap();
    for table in EVENTS_TABLES {
        assert_eq!(reason(&store, table.index()), "the capture lost the interval", "{table:?} unavailable for the log's reason");
    }
    end(&mut store);
    log_hour(&mut store, 5, &[], &[], Status::Complete);
    writers.hour(1, 5, false, &mut store).unwrap();
    assert_eq!(statuses(&store), [Status::Complete; 2]);
    end(&mut store);

    let hand = |hour: i32, appended: &[(u64, u128, u64)], backlog: bool, forgotten: &[std::ops::Range<u64>]| Handed {
        hour,
        appended: appended.to_vec(),
        backlog,
        forgotten: forgotten.to_vec(),
    };
    let expected = [
        hand(0, &[(0, 1, 10), (1, 2, 20), (2, 3, 30)], true, &[]),
        hand(1, &[(3, 4, 40)], false, &[0..1]),
        hand(3, &[(4, 5, 50)], false, &[]),
        hand(4, &[], false, &[]),
        hand(5, &[], false, &[]),
    ];
    assert_eq!(*events.lock().unwrap(), expected, "no backlog after the run's first hour");
}

#[test]
fn an_update_in_place_is_a_forget_and_an_append_of_the_same_key_in_one_hour() {
    let handed = Handings::default();
    let mut store = log_store();
    let mut writers = Writers::new(vec![follow("Keyed", &EVENTS_OUTPUTS, EVENTS, &handed)]);
    writers.start_run();
    store.begin_run(0);
    log_hour(&mut store, 0, &[(7, 1), (8, 2)], &[], Status::Complete);
    writers.hour(1, 0, true, &mut store).unwrap();
    store.end_hour(&mut replay_columnar::unit::Discard).unwrap();
    log_hour(&mut store, 1, &[(7, 5)], &[0], Status::Complete);
    writers.hour(1, 1, false, &mut store).unwrap();
    let last = handed.lock().unwrap().last().cloned().unwrap();
    assert_eq!((last.appended, last.forgotten), (vec![(2, 7, 5)], vec![0..1]), "one new row and the old ordinal forgotten");
    let by_key = store.view(EVENTS_TABLES[0].index());
    let cells = |key: u128| {
        let slot = by_key.slot(key).unwrap();
        (key, by_key.number(0, slot), by_key.number(1, slot))
    };
    let rows: Vec<_> = keys_of(&store, EVENTS_TABLES[0]).into_iter().map(cells).collect();
    assert_eq!(rows, [(7, Some(2), Some(5)), (8, Some(1), Some(2))], "the key holds the new ordinal and cells once");
    assert_eq!(keys_of(&store, EVENTS_TABLES[1]), [1, 2], "the old ordinal gone");
}

#[test]
fn a_log_read_otherwise_than_by_its_appended_rows_is_refused_by_name() {
    const CHANGED: &[Read] = &[Read { table: LOG, columns: Columns::Every, watch: Watch::Changed, required: false }];
    const STATE: &[Read] = &[Read { table: TableId::Countries, columns: Columns::Every, watch: Watch::Appended, required: false }];
    let handed = Handings::default();
    for (reads, refusal) in [
        (CHANGED, "Wrong reads log table MarketEvents with watch Changed: a log is read by its appended rows"),
        (STATE, "Wrong reads state table Countries with watch Appended: only a log is read by its appended rows"),
    ] {
        let mut store = log_store();
        let mut writers = Writers::new(vec![follow("Wrong", &EVENTS_OUTPUTS, reads, &handed)]);
        writers.start_run();
        store.begin_run(0);
        log_hour(&mut store, 0, &[(1, 1)], &[], Status::Complete);
        let refused = writers.hour(1, 0, true, &mut store).unwrap_err();
        assert_eq!((refused.kind(), refused.to_string()), (io::ErrorKind::InvalidInput, refusal.to_owned()));
    }
    assert!(handed.lock().unwrap().is_empty(), "no writer ran");
}

mod counting {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
    };

    #[cfg(feature = "memory-labels")]
    const INNER: engine_allocation::labels::Counting<System> = engine_allocation::labels::Counting(System);
    #[cfg(not(feature = "memory-labels"))]
    const INNER: System = System;

    thread_local! {
        static HELD: Cell<isize> = const { Cell::new(0) };
        static PEAK: Cell<isize> = const { Cell::new(0) };
    }

    pub struct Counting;

    fn moved(by: isize) {
        let _ = HELD.try_with(|held| {
            let now = held.get() + by;
            held.set(now);
            let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
        });
    }

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let at = unsafe { INNER.alloc(layout) };
            if !at.is_null() {
                moved(layout.size() as isize);
            }
            at
        }

        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let at = unsafe { INNER.alloc_zeroed(layout) };
            if !at.is_null() {
                moved(layout.size() as isize);
            }
            at
        }

        unsafe fn dealloc(&self, at: *mut u8, layout: Layout) {
            unsafe { INNER.dealloc(at, layout) };
            moved(-(layout.size() as isize));
        }

        unsafe fn realloc(&self, at: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            let grown = unsafe { INNER.realloc(at, layout, size) };
            if !grown.is_null() {
                moved(size as isize);
                moved(-(layout.size() as isize));
            }
            grown
        }
    }

    pub fn peak(run: impl FnOnce()) -> usize {
        let before = HELD.with(Cell::get);
        PEAK.with(|peak| peak.set(before));
        run();
        (PEAK.with(Cell::get) - before).max(0) as usize
    }
}

#[global_allocator]
static COUNTING: counting::Counting = counting::Counting;

#[test]
fn a_writer_setting_every_row_it_holds_again_holds_no_copy_of_them() {
    const ROWS: usize = 200_000;
    let mut store = store();
    let rows = (0..ROWS as u128).map(|key| (key, Value::Bytes(0))).collect();
    let mut writers = Writers::new(vec![Box::new(Setting { rows })]);
    writers.start_run();
    let campaign = TableId::Campaign.index();
    store.begin_hour(0, &[b"a"]).unwrap();
    writers.hour(1, 0, true, &mut store).unwrap();
    assert_eq!(store.view(campaign).rows().count(), ROWS);
    store.begin_hour(1, &[]).unwrap();
    let peak = counting::peak(|| writers.hour(1, 1, false, &mut store).unwrap());
    let log = &store.log().tables[campaign];
    assert!(log.inserted.is_empty() && log.written.is_empty(), "every row set equal: {log:?}");
    assert!(peak < ROWS * 4, "{peak} bytes held at once to set {ROWS} rows equal");
}

struct SetsThenFails {
    fails: i32,
}

impl Writer for SetsThenFails {
    fn name(&self) -> &'static str {
        "SetsThenFails"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::Campaign)]
    }

    fn reads(&self) -> &'static [Read] {
        &[]
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {}

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()> {
        if hour.hour < self.fails {
            out.set(TableId::Campaign, 1, vec![Value::Bytes(0)]);
            out.set(TableId::Campaign, 3, vec![Value::Bytes(0)]);
            return Ok(());
        }
        out.set(TableId::Campaign, 1, vec![Value::Bytes(1)]);
        let made = out.text(b"made up");
        out.set(TableId::Campaign, 2, vec![made]);
        out.delete(TableId::Campaign, 3);
        Err(invalid("it broke midway"))
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_writer_failing_after_it_set_rows_leaves_its_table_as_it_was_and_unavailable() {
    let mut store = store();
    let mut writers = Writers::new(vec![Box::new(SetsThenFails { fails: 1 })]);
    writers.start_run();
    let campaign = TableId::Campaign.index();
    store.begin_hour(0, &[b"a", b"b"]).unwrap();
    writers.hour(1, 0, true, &mut store).unwrap();
    let before = rows_of(&store, TableId::Campaign);
    let slots = store.view(campaign).state().slots();
    store.begin_hour(1, &[]).unwrap();
    writers.hour(1, 1, false, &mut store).unwrap();
    assert!(reason(&store, campaign).contains("the recorder stopped working this out: it broke midway"), "{}", reason(&store, campaign));
    assert_eq!(rows_of(&store, TableId::Campaign), before);
    assert_eq!(before, [(1, Some(b"a".to_vec())), (3, Some(b"a".to_vec()))]);
    assert_eq!(store.view(campaign).state().slots(), slots);
    let log = &store.log().tables[campaign];
    assert!(log.inserted.is_empty() && log.deleted.is_empty() && log.written.is_empty(), "{log:?}");
    assert!(writers.save().unwrap().windows(7).all(|text| text != b"made up"), "the failed hour's text is not named");
}

struct Misbehaving {
    twice: bool,
}

impl Writer for Misbehaving {
    fn name(&self) -> &'static str {
        "Misbehaving"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::Campaign)]
    }

    fn reads(&self) -> &'static [Read] {
        &[]
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {}

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()> {
        if hour.hour == 0 {
            out.set(TableId::Campaign, 1, vec![Value::Bytes(0)]);
            return Ok(());
        }
        if self.twice {
            out.set(TableId::Campaign, 2, vec![Value::Bytes(0)]);
            out.set(TableId::Campaign, 2, vec![Value::Bytes(1)]);
        } else {
            let held = hour.table(TableId::Campaign).rows().count();
            out.set(TableId::Campaign, 2, vec![Value::Bytes(held as u32)]);
        }
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_writer_setting_a_row_twice_or_reading_its_own_table_through_the_hour_fails_by_name() {
    for (twice, refusal) in [
        (true, "the recorder stopped working this out: it set one campaign row twice in one hour"),
        (false, "the recorder stopped working this out: it read its own campaign as an input"),
    ] {
        let mut store = store();
        let mut writers = Writers::new(vec![Box::new(Misbehaving { twice })]);
        writers.start_run();
        store.begin_hour(0, &[b"a", b"b"]).unwrap();
        writers.hour(1, 0, true, &mut store).unwrap();
        store.begin_hour(1, &[]).unwrap();
        writers.hour(1, 1, false, &mut store).unwrap();
        assert_eq!(reason(&store, TableId::Campaign.index()), refusal);
        assert_eq!(rows_of(&store, TableId::Campaign), [(1, Some(b"a".to_vec()))]);
    }
}

struct Ordered {
    order: fn(&mut Vec<u128>),
    held: Vec<u128>,
}

impl Writer for Ordered {
    fn name(&self) -> &'static str {
        "Ordered"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(ORDERED)]
    }

    fn reads(&self) -> &'static [Read] {
        &[]
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {
        self.held.clear();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()> {
        let at = hour.hour as u128;
        let texts = [&b"north"[..], b"south", b"east"].map(|text| out.text(text));
        let alive: Vec<u128> = (0..48).filter(|key| (key * 7 + at) % 5 != 0).collect();
        let mut keys = alive.clone();
        (self.order)(&mut keys);
        for key in keys {
            let opaque = out.opaque(vec![key as u8; ((key + at) % 3) as usize]);
            let optional = match (key + at) % 6 {
                0 => Value::Unrecorded,
                1 => Value::OptionalI64(None),
                _ => Value::OptionalI64(Some(key as i64 - at as i64)),
            };
            out.set(ORDERED, key, vec![Value::U64((key * at % 3) as u64), optional, texts[((key + at / 4) % 3) as usize], opaque]);
        }
        let mut gone: Vec<u128> = self.held.iter().copied().filter(|key| !alive.contains(key)).collect();
        (self.order)(&mut gone);
        for key in gone {
            out.delete(ORDERED, key);
        }
        out.delete(ORDERED, alive[0]);
        self.held = alive;
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

type Written = (Vec<u8>, Vec<replay_columnar::unit::BlockPage>, Vec<Option<u128>>);

fn ordered_unit(order: fn(&mut Vec<u128>)) -> Written {
    let kinds = schema();
    let mut store = Store::new(kinds.clone(), vec![false; TABLE_COUNT]);
    store.attach(replay_columnar::unit::Encoder::new(&kinds, replay_columnar::unit::Settings::at(3)).unwrap());
    let mut writers = Writers::new(vec![Box::new(Ordered { order, held: Vec::new() })]);
    writers.start_run();
    let mut unit = Memory::default();
    store.begin_run(0);
    for hour in 0..30 {
        store.begin_hour(hour, &[]).unwrap();
        writers.hour(1, hour, hour == 0, &mut store).unwrap();
        store.end_hour(&mut unit).unwrap();
    }
    store.end_run(&mut unit).unwrap();
    let table = store.view(ORDERED.index());
    let keys = (0..table.state().slots()).map(|slot| table.key(slot)).collect();
    (unit.bytes, unit.pages, keys)
}

#[test]
fn a_writer_setting_its_rows_in_any_order_writes_the_bytes_of_one_setting_them_in_key_order() {
    let ascending = ordered_unit(|keys| keys.sort_unstable());
    assert!(!ascending.0.is_empty() && ascending.2.iter().any(Option::is_some));
    let scrambled: fn(&mut Vec<u128>) = |keys| keys.sort_unstable_by_key(|key| (key * 29 + 11) % 48);
    let descending: fn(&mut Vec<u128>) = |keys| keys.sort_unstable_by(|a, b| b.cmp(a));
    for order in [scrambled, descending] {
        let written = ordered_unit(order);
        assert!(written.0 == ascending.0, "the unit's bytes differ");
        assert_eq!((written.1, written.2), (ascending.1.clone(), ascending.2.clone()));
    }
}

type Ran = std::sync::Arc<std::sync::Mutex<Vec<(i32, bool)>>>;

const EVERY_COUNTRIES: &[Read] = &[Read { table: TableId::Countries, columns: Columns::Every, watch: Watch::EveryRead, required: false }];

struct EveryReader {
    table: &'static [Output],
    hourly: bool,
    ran: Ran,
}

impl Writer for EveryReader {
    fn name(&self) -> &'static str {
        if self.hourly { "HourlyEveryReader" } else { "EveryReader" }
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        self.table
    }

    fn reads(&self) -> &'static [Read] {
        EVERY_COUNTRIES
    }

    fn hourly(&self) -> bool {
        self.hourly
    }

    fn start(&mut self) {}

    fn hour(&mut self, hour: &Hour<'_>, _: &mut Rows) -> io::Result<()> {
        let read = hour.table(TableId::Countries).sampled_at() == Some(hour.hour);
        self.ran.lock().unwrap().push((hour.hour, read));
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

fn unchanged(journal: &mut replay_columnar::store::journal::Journal, store: &mut Store, hour: i32, read: bool) {
    let mut statuses = vec![Status::NotRecorded; TABLE_COUNT];
    statuses[TableId::Countries.index()] = Status::Complete;
    statuses[TableId::CountryLinks.index()] = Status::Complete;
    let mut bits = vec![0u64; TABLE_COUNT.div_ceil(64)];
    if read {
        let table = TableId::Countries.index();
        bits[table / 64] |= 1 << (table % 64);
    }
    let manifest = replay_columnar::manifest::Manifest {
        branch: 1,
        hour,
        flags: 0,
        total_ns: 0,
        pool_bytes: 0,
        dropped_frames: 0,
        appends: Vec::new(),
        statuses,
        read: bits,
        samples: Vec::new(),
        sections: Vec::new(),
    };
    journal.begin_hour(store, &manifest).unwrap();
    journal.applied(store);
}

#[test]
fn a_writer_watching_every_read_runs_when_the_input_is_stamped_read_and_not_when_it_is_not() {
    let (gated, every) = (Ran::default(), Ran::default());
    let mut store = store();
    let mut journal = replay_columnar::store::journal::Journal::leaving(&[TableId::TagNames.index(), TableId::TagHolders.index()], Default::default());
    let mut writers = Writers::new(vec![
        Box::new(EveryReader { table: &[Output::State(TableId::TagNames)], hourly: false, ran: gated.clone() }),
        Box::new(EveryReader { table: &[Output::State(TableId::TagHolders)], hourly: true, ran: every.clone() }),
    ]);
    writers.start_run();
    let countries = TableId::Countries.index();
    apply(&mut store, 0, &[(9, 3, 0)], &[(9, 3)], Status::Complete);
    writers.hour(1, 0, true, &mut store).unwrap();
    for (hour, read) in [(1, true), (2, false), (3, true)] {
        unchanged(&mut journal, &mut store, hour, read);
        assert_eq!(store.view(countries).status(), Status::Complete, "hour {hour}");
        assert_eq!(store.view(countries).sampled_at(), Some(if read { hour } else { hour - 1 }), "hour {hour}: the journal stamps a read with no section, and only a read");
        assert_eq!(moved(&store, &EVERY_COUNTRIES[0], hour), read, "hour {hour}: Countries moved as read every read");
        writers.hour(1, hour, false, &mut store).unwrap();
    }
    assert_eq!(*gated.lock().unwrap(), [(0, true), (1, true), (3, true)], "run by its reads: not in the hour that did not read Countries");
    assert_eq!(*every.lock().unwrap(), [(0, true), (1, true), (2, false), (3, true)], "run every hour, told hour 2 did not read Countries");
}

const GRACE: definitions::Define = definitions::Define { section: "NMilitary", name: "SUPPLY_GRACE", form: definitions::Form::Int32 };
const LOW: definitions::Define = definitions::Define { section: "NMilitary", name: "LOW_SUPPLY", form: definitions::Form::Fixed5 };
const VERY_LOW_ICON: definitions::Define = definitions::Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD", form: definitions::Form::Fixed5 };
const LOW_ICON: definitions::Define = definitions::Define { section: "NGraphics", name: "SUPPLY_UNIT_COUNTER_LOW_THRESHOLD", form: definitions::Form::Fixed5 };

struct Thresholds {
    seen: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Thresholds {
    fn note(&self, what: String) {
        self.seen.lock().unwrap().push(what);
    }
}

impl Writer for Thresholds {
    fn name(&self) -> &'static str {
        "Thresholds"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::CountryStability)]
    }

    fn reads(&self) -> &'static [Read] {
        &[]
    }

    fn hourly(&self) -> bool {
        true
    }

    fn definitions(&self) -> &'static [Uses] {
        &[
            Uses { definition: Definition::Define(GRACE), required: true },
            Uses { definition: Definition::Define(LOW), required: true },
            Uses { definition: Definition::Define(VERY_LOW_ICON), required: true },
            Uses { definition: Definition::Define(LOW_ICON), required: true },
            Uses { definition: Definition::SubUnitNeeds, required: false },
        ]
    }

    fn start(&mut self) {}

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let held = hour.definitions();
        let define = |define: definitions::Define| held.define(define.section, define.name).map_err(|why| invalid(why.to_owned()));
        out.set(TableId::CountryStability, 0, vec![Value::I64(define(GRACE)?), Value::I64(define(LOW)?), Value::I64(define(VERY_LOW_ICON)? + define(LOW_ICON)?)]);
        let need = held.needs().ok().and_then(|needs| needs.get("Infantry")).map(<[(String, i64)]>::to_vec);
        self.note(format!("hour {} fresh {}: need {need:?}", hour.hour, hour.fresh));
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        self.note(format!("primed with low supply {:?}", hour.definitions().define("NMilitary", "LOW_SUPPLY")));
    }
}

fn thresholds() -> Definitions {
    let needs = content_reader_key_names::unit_leader::needs::SubUnitNeeds {
        needs: [("infantry".to_owned(), vec![("infantry_equipment".to_owned(), 10_000_000)])].into(),
        dropped: Vec::new(),
    };
    Definitions::read_none()
        .with_define(GRACE, Ok(240))
        .with_define(LOW, Ok(25_000))
        .with_define(VERY_LOW_ICON, Ok(10_000))
        .with_define(LOW_ICON, Ok(50_000))
        .with_needs(Ok(needs))
}

fn stability_row(store: &Store) -> Vec<Option<u64>> {
    let table = store.view(TableId::CountryStability.index());
    let slot = table.slot(0).unwrap();
    (0..3)
        .map(|column| match table.value(column, slot) {
            replay_columnar::store::Value::Number(bits) => Some(bits),
            _ => None,
        })
        .collect()
}

fn identity() -> Identity {
    Identity {
        game_version: "synthetic".into(),
        exe_hash: [1; 32],
        exe_size: 1,
        vanilla_hash: [2; 32],
        dlc: vec![],
        mods: vec![],
        coverage_policy: 1,
        language: "english".into(),
        recorder_version: "test".into(),
        collector_hash: [3; 32],
        verification: replay_data::identity::Verification::Unchecked,
    }
}

#[test]
fn definitions_reach_the_writers_declaring_them_and_absent_ones_withhold_by_name() {
    let mut store = store();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut writers = Writers::new(vec![Box::new(Thresholds { seen: Arc::clone(&seen) })]);
    writers.start_run();
    let stability = TableId::CountryStability.index();
    store.begin_hour(0, &[]).unwrap();
    writers.hour(1, 0, true, &mut store).unwrap();
    let why = reason(&store, stability);
    assert!(why.contains("the game's definition define NMilitary.SUPPLY_GRACE is not held: the session's game facts are not packed yet"), "{why}");
    assert!(!why.contains("Thresholds"), "a writer's identifier never reaches a reason: {why}");
    assert_eq!(store.view(stability).rows().count(), 0, "no row is written while the definitions are absent");
    assert!(seen.lock().unwrap().is_empty(), "the writer was not run");

    writers.define(thresholds());
    store.begin_hour(1, &[]).unwrap();
    writers.hour(1, 1, false, &mut store).unwrap();
    assert_eq!(store.view(stability).status(), Status::Complete);
    assert_eq!(stability_row(&store), [Some(240), Some(25_000), Some(60_000)]);
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        ["hour 1 fresh true: need Some([(\"infantry_equipment\", 10000000)])"]
    );
    writers.define(thresholds());
    store.begin_hour(2, &[]).unwrap();
    writers.hour(1, 2, false, &mut store).unwrap();
    assert!(seen.lock().unwrap()[1].starts_with("hour 2 fresh false"), "{:?}", seen.lock().unwrap());

    let saved = writers.save().unwrap();
    let resumed_seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut resumed = Writers::new(vec![Box::new(Thresholds { seen: Arc::clone(&resumed_seen) })]);
    resumed.define(thresholds());
    resumed.load(&saved, &mut store).unwrap();
    resumed.prime(&store, 1, 2);
    store.begin_hour(3, &[]).unwrap();
    resumed.hour(1, 3, false, &mut store).unwrap();
    assert_eq!(store.view(stability).status(), Status::Complete);
    assert_eq!(stability_row(&store), [Some(240), Some(25_000), Some(60_000)]);
    let resumed_seen = resumed_seen.lock().unwrap();
    assert_eq!(resumed_seen[0], "primed with low supply Ok(25000)");
    assert!(resumed_seen[1].starts_with("hour 3 fresh false"), "{resumed_seen:?}");
}

#[test]
fn definitions_are_read_from_a_pack_for_the_kinds_writers_declare() {
    fn reader(facts: &Facts, _: &Identity, wanted: &[Definition]) -> Definitions {
        let mut pack = [0; 4];
        facts.read_at(0, &mut pack).unwrap();
        assert_eq!((&pack, facts.length), (b"pack", 4));
        assert_eq!(wanted.len(), 5);
        assert!(wanted.contains(&Definition::Define(LOW_ICON)) && wanted.contains(&Definition::SubUnitNeeds));
        assert!(!wanted.contains(&Definition::ProvinceAreas));
        thresholds()
    }
    let mut writers = Writers::new(vec![Box::new(Thresholds { seen: Arc::default() })]);
    writers.read_definitions_with(reader);
    writers.define_from(&Facts::held(Arc::new(b"pack".to_vec())), &identity());
    assert_eq!(writers.definitions().define("NGraphics", "SUPPLY_UNIT_COUNTER_LOW_THRESHOLD"), Ok(50_000));
    let mut none = Writers::new(vec![Box::new(Setting { rows: Vec::new() })]);
    none.read_definitions_with(|_, _, _| panic!("nothing is read when no writer declares a definition"));
    none.define_from(&Facts::held(Arc::new(b"pack".to_vec())), &identity());
}

type Calls = std::sync::Arc<std::sync::Mutex<Vec<(i32, bool)>>>;

struct Called {
    calls: Calls,
    hourly: bool,
}

impl Writer for Called {
    fn name(&self) -> &'static str {
        "Called"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::Campaign)]
    }

    fn reads(&self) -> &'static [Read] {
        &[Read { table: TableId::Countries, columns: Columns::Every, watch: Watch::Changed, required: true }]
    }

    fn hourly(&self) -> bool {
        self.hourly
    }

    fn start(&mut self) {}

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()> {
        self.calls.lock().unwrap().push((hour.hour, hour.fresh));
        out.status(TableId::Campaign, Status::Truncated { missing: 1 });
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_writer_withheld_for_its_input_says_why_in_plain_words() {
    let mut store = store();
    let mut writers = Writers::new(vec![Box::new(Called { calls: Calls::default(), hourly: false })]);
    writers.start_run();
    let lost = store.intern(b"the capture lost the interval").unwrap();
    apply(&mut store, 0, &[(9, 3, 0)], &[(9, 3)], Status::Unavailable { reason: lost });
    writers.hour(1, 0, true, &mut store).unwrap();
    assert_eq!(reason(&store, TableId::Campaign.index()), "countries: not available (the capture lost the interval)");
}

fn called(hourly: bool) -> (Vec<(i32, bool)>, Vec<(Status, Option<i32>)>) {
    let calls = Calls::default();
    let mut store = store();
    let mut writers = Writers::new(vec![Box::new(Called { calls: calls.clone(), hourly })]);
    writers.start_run();
    let (countries, links) = ([(9, 3, 0), (4, 5, 1)], [(9, 3), (4, 5)]);
    let campaign = TableId::Campaign.index();
    let mut statuses = Vec::new();
    for hour in 0..7 {
        match hour {
            0 | 2 | 5 => apply(&mut store, hour, &countries, &links, Status::Complete),
            4 => apply(&mut store, hour, &countries, &links, Status::Unavailable { reason: 0 }),
            _ => {
                store.begin_hour(hour, &[]).unwrap();
                store.settle();
            }
        }
        writers.hour(1, hour, hour == 0, &mut store).unwrap();
        statuses.push((store.view(campaign).status(), store.view(campaign).sampled_at()));
    }
    let calls = calls.lock().unwrap().clone();
    (calls, statuses)
}

#[test]
fn a_writer_reading_state_tables_is_called_only_in_an_hour_that_moved_them() {
    let (calls, statuses) = called(false);
    assert_eq!(calls, [(0, true), (2, false), (5, true)]);
    let told = Status::Truncated { missing: 1 };
    assert_eq!(statuses[1], (told, Some(1)), "an hour it is not called keeps the status it told");
    assert_eq!(statuses[3], (told, Some(3)));
    assert!(matches!(statuses[4].0, Status::Unavailable { .. }), "{:?}", statuses[4]);
    assert_eq!(statuses[6], (told, Some(6)));
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Toyed {
    appended: Vec<(i32, u64, u128, Vec<String>)>,
    forgotten: Vec<(i32, u64)>,
}

impl Toyed {
    fn sorted(mut self) -> Self {
        self.forgotten.sort_unstable();
        self
    }

    fn at(&self, hour: i32) -> (u64, Vec<u64>) {
        let appended: Vec<u64> = self.appended.iter().filter(|row| row.0 <= hour).map(|row| row.1).collect();
        let forgotten: std::collections::HashSet<u64> = self.forgotten.iter().filter(|row| row.0 <= hour).map(|row| row.1).collect();
        (appended.len() as u64, appended.into_iter().filter(|ordinal| !forgotten.contains(ordinal)).collect())
    }
}

type Toys = std::sync::Arc<std::sync::Mutex<Toyed>>;

const TOY_RUN: &[Output] = &[Output::Log(TOY, Keys::Run)];
const TOY_HOURLY: &[Output] = &[Output::Log(TOY, Keys::Hour(32))];

struct Toy {
    pending: Vec<u64>,
    toyed: Toys,
    outputs: &'static [Output],
}

fn toy(toyed: &Toys, outputs: &'static [Output]) -> Box<dyn Writer> {
    Box::new(Toy { pending: Vec::new(), toyed: toyed.clone(), outputs })
}

impl Writer for Toy {
    fn name(&self) -> &'static str {
        "Toy"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        self.outputs
    }

    fn reads(&self) -> &'static [Read] {
        &[]
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {
        self.pending.clear();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()> {
        use replay_columnar::store::Value as Held;
        let at = hour.hour;
        let mut toyed = self.toyed.lock().unwrap();
        let forgot = std::mem::take(&mut self.pending);
        for &ordinal in &forgot {
            out.forget(TOY, ordinal);
            toyed.forgotten.push((at, ordinal));
        }
        let texts = [out.text(b"north"), out.text(b"south")];
        let count = at % 5 + i32::from(at % 97 == 0);
        for index in 0..count {
            let seed = at as u64 * 7 + index as u64;
            let opaque = vec![seed as u8; 1 + (seed % 3) as usize];
            let optional = match seed % 3 {
                0 => Value::Unrecorded,
                1 => Value::OptionalI32(None),
                _ => Value::OptionalI32(Some(-(seed as i32))),
            };
            let text = texts[(seed % 2) as usize];
            let Value::Bytes(id) = text else { panic!("a text cell") };
            let shown = [
                Held::Number(seed),
                Held::Text(id),
                Held::List(&opaque),
                match optional {
                    Value::OptionalI32(Some(value)) => Held::Number(value as i64 as u64),
                    Value::OptionalI32(None) => Held::Missing,
                    _ => Held::Unrecorded,
                },
            ]
            .map(|cell| format!("{cell:?}"));
            let cells = vec![Value::U64(seed), text, out.opaque(opaque.clone()), optional];
            let key = u128::from(at as u32) << 32 | u128::from(index as u32);
            let ordinal = out.append(TOY, key, cells).expect("the toy's append");
            toyed.appended.push((at, ordinal, key, shown.to_vec()));
            if at % 97 == 0 && index + 1 == count {
                out.forget(TOY, ordinal);
                toyed.forgotten.push((at, ordinal));
            } else if index % 3 == 0 {
                self.pending.push(ordinal);
            }
        }
        let (_, listed) = hour.listed(TOY).expect("the toy's log");
        let older: Vec<u64> = listed.ones().filter(|ordinal| !forgot.contains(ordinal)).collect();
        for &ordinal in older.iter().take(older.len().saturating_sub(40)) {
            out.forget(TOY, ordinal);
            toyed.forgotten.push((at, ordinal));
        }
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        blob.varint(self.pending.len() as u64)?;
        for &ordinal in &self.pending {
            blob.varint(ordinal)?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        let mut reader = Reader::new(blob);
        let count = reader.count()?;
        self.pending = (0..count).map(|_| reader.varint()).collect::<io::Result<_>>()?;
        reader.finish()
    }
}

fn toy_store(encoded: bool) -> Store {
    let kinds = schema();
    let mut store = Store::new(kinds.clone(), vec![false; TABLE_COUNT]);
    let mut logs = vec![false; TABLE_COUNT];
    logs[TOY.index()] = true;
    store.declare_logs(&logs).unwrap();
    if encoded {
        store.attach(replay_columnar::unit::Encoder::new(&kinds, replay_columnar::unit::Settings::at(3)).unwrap());
    }
    store
}

struct Commit {
    length: usize,
    pages: usize,
    encoder: Vec<u8>,
    writers: Vec<u8>,
    next: u32,
}

fn toy_hours(store: &mut Store, writers: &mut Writers, hours: std::ops::Range<u32>, unit: &mut Memory) -> Option<Commit> {
    use replay_columnar::unit::BLOCK;
    let mut commit = None;
    for at in hours {
        store.begin_hour(at as i32, &[]).unwrap();
        writers.hour(1, at as i32, at == 0, store).unwrap();
        store.end_hour(unit).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
            commit = Some(Commit {
                length: unit.bytes.len(),
                pages: unit.pages.len(),
                encoder: store.encoder().unwrap().save(),
                writers: writers.save().unwrap(),
                next: store.strings().next(),
            });
        }
    }
    commit
}

fn read_toy(unit: &Memory) -> (Toyed, HashMap<u32, Vec<u8>>) {
    use replay_columnar::unit::reader::{LogEvent, log_span};
    let columns: Vec<usize> = (0..TOY_KINDS.len()).collect();
    let (mut read, mut named) = (Toyed::default(), HashMap::new());
    log_span(&unit.bytes, &unit.pages, &TOY_KINDS, TOY.index(), &columns, 0, 0..=i32::MAX, &mut |hour, event| match event {
        LogEvent::Appended { ordinal, key, rows, at, .. } => read.appended.push((hour, ordinal, key, columns.iter().map(|&column| format!("{:?}", rows.value(column, at))).collect())),
        LogEvent::Forgotten { from, to, .. } => read.forgotten.extend((from..to).map(|ordinal| (hour, ordinal))),
        LogEvent::Named { id, text } => {
            named.insert(id, text.to_vec());
        }
    })
    .unwrap();
    (read, named)
}

fn seek_toy(unit: &Memory, hour: u32) -> (u64, Vec<u64>) {
    use replay_columnar::unit::{
        BLOCK,
        reader::{LogEvent, log_span, log_state},
    };
    let block = hour / BLOCK;
    let (mut count, mut listed) = match block {
        0 => (0, Listed::default()),
        _ => {
            let restored = log_state(&unit.bytes, &unit.pages, &TOY_KINDS, TOY.index(), block as usize - 1).unwrap();
            (restored.count().unwrap(), restored.listed().unwrap().clone())
        }
    };
    let columns: Vec<usize> = (0..TOY_KINDS.len()).collect();
    log_span(&unit.bytes, &unit.pages, &TOY_KINDS, TOY.index(), &columns, 0, (block * BLOCK) as i32..=hour as i32, &mut |_, event| match event {
        LogEvent::Appended { ordinal, .. } => {
            count += 1;
            listed.append(ordinal);
        }
        LogEvent::Forgotten { from, to, .. } => listed.clear_range(from, to),
        LogEvent::Named { .. } => {}
    })
    .unwrap();
    (count, listed.ones().collect())
}

#[test]
fn a_writer_s_log_restores_at_every_block_and_seeks_to_any_hour_as_it_appended_and_forgot() {
    use replay_columnar::unit::{BLOCK, reader::log_state};
    for outputs in [TOY_RUN, TOY_HOURLY] {
        let toys = Toys::default();
        let mut store = toy_store(true);
        let mut writers = Writers::new(vec![toy(&toys, outputs)]);
        writers.start_run();
        store.begin_run(0);
        let mut unit = Memory::default();
        toy_hours(&mut store, &mut writers, 0..2 * BLOCK, &mut unit).expect("a commit at each block's end");
        let toyed = toys.lock().unwrap().clone().sorted();
        assert_eq!(unit.pages.len(), 2, "{outputs:?}");
        assert!(unit.pages.iter().all(|page| page.tables[TOY.index()].log), "{outputs:?}: the toy table is a log in every block");
        assert!(toyed.forgotten.contains(&(0, 0)), "{outputs:?}: the run's first hour forgets the row it appended");
        let crossing = toyed.forgotten.iter().filter(|&&(hour, ordinal)| hour >= BLOCK as i32 && toyed.appended.iter().any(|row| row.1 == ordinal && row.0 < BLOCK as i32)).count();
        assert!(crossing > 0, "{outputs:?}: rows appended in the first block are forgotten in the second");
        let (read, named) = read_toy(&unit);
        assert_eq!(read.appended.len(), toyed.appended.len(), "{outputs:?}");
        assert_eq!(read, toyed, "{outputs:?}: every row and forget read back in its hour");
        let mut texts: Vec<Vec<u8>> = named.into_values().collect();
        texts.sort_unstable();
        assert_eq!(texts, [b"north".to_vec(), b"south".to_vec()], "{outputs:?}: the strings the rows name read back");
        for at in 0..unit.pages.len() {
            let restored = log_state(&unit.bytes, &unit.pages, &TOY_KINDS, TOY.index(), at).unwrap();
            let (count, listed) = toyed.at(((at as u32 + 1) * BLOCK - 1) as i32);
            assert_eq!((restored.count(), restored.listed().unwrap().ones().collect::<Vec<_>>()), (Some(count), listed), "{outputs:?}: block {at}'s restore");
            assert_eq!(restored.status(), Status::Complete, "{outputs:?}");
        }
        for hour in 0..2 * BLOCK {
            assert_eq!(seek_toy(&unit, hour), toyed.at(hour as i32), "{outputs:?}: a seek to hour {hour}");
        }
    }
}

#[test]
fn a_writer_s_log_taken_up_at_a_block_s_end_writes_what_an_unbroken_run_writes() {
    use replay_columnar::{
        store::TableState,
        unit::{BLOCK, reader::log_state},
    };
    for outputs in [TOY_RUN, TOY_HOURLY] {
        let mut unbroken = Memory::default();
        {
            let mut store = toy_store(true);
            let mut writers = Writers::new(vec![toy(&Toys::default(), outputs)]);
            writers.start_run();
            store.begin_run(0);
            toy_hours(&mut store, &mut writers, 0..2 * BLOCK, &mut unbroken);
        }
        let mut unit = Memory::default();
        let commit = {
            let mut store = toy_store(true);
            let mut writers = Writers::new(vec![toy(&Toys::default(), outputs)]);
            writers.start_run();
            store.begin_run(0);
            toy_hours(&mut store, &mut writers, 0..BLOCK, &mut unit).expect("a commit at the first block's end")
        };
        unit.bytes.truncate(commit.length);
        unit.pages.truncate(commit.pages);
        let kinds = schema();
        let mut tables: Vec<TableState> = kinds.iter().map(|kinds| TableState::new(kinds)).collect();
        tables[TOY.index()] = log_state(&unit.bytes, &unit.pages, &TOY_KINDS, TOY.index(), commit.pages - 1).unwrap();
        let mut store = toy_store(true);
        store.begin_run(0);
        store.encoder_mut().unwrap().load(&commit.encoder).unwrap();
        store.resume(tables, commit.next).unwrap();
        let mut writers = Writers::new(vec![toy(&Toys::default(), outputs)]);
        writers.start_run();
        writers.load(&commit.writers, &mut store).unwrap();
        assert_eq!(writers.withheld, [None], "{outputs:?}: the toy is taken up, not withheld");
        writers.prime(&store, 1, BLOCK as i32 - 1);
        toy_hours(&mut store, &mut writers, BLOCK..2 * BLOCK, &mut unit);
        assert_eq!(unit.pages, unbroken.pages, "{outputs:?}");
        assert!(unit.bytes == unbroken.bytes, "{outputs:?}: the resumed unit differs");
    }
}

#[test]
fn a_log_keyed_by_its_hour_holds_no_key_past_its_hour_and_its_block_end_state_none() {
    use replay_columnar::unit::BLOCK;
    let commits = |outputs: &'static [Output]| {
        let mut store = toy_store(true);
        let mut writers = Writers::new(vec![toy(&Toys::default(), outputs)]);
        writers.start_run();
        store.begin_run(0);
        let mut unit = Memory::default();
        [0..BLOCK, BLOCK..2 * BLOCK].map(|hours| {
            let commit = toy_hours(&mut store, &mut writers, hours, &mut unit).expect("a commit at the block's end");
            let held: usize = writers.given[0].iter().map(replay_columnar::store::Given::len).sum();
            let mut back = Writers::new(vec![toy(&Toys::default(), outputs)]);
            back.load(&commit.writers, &mut toy_store(false)).unwrap();
            let saved: usize = back.given[0].iter().map(replay_columnar::store::Given::len).sum();
            (commit.writers, held, saved)
        })
    };
    let [(run_first, run_held_first, run_saved_first), (run_second, run_held_second, run_saved_second)] = commits(TOY_RUN);
    let [(hourly_first, hourly_held_first, hourly_saved_first), (hourly_second, hourly_held_second, hourly_saved_second)] = commits(TOY_HOURLY);
    assert_eq!(run_saved_first, run_held_first);
    assert_eq!(run_saved_second, run_held_second);
    assert!(run_held_first > BLOCK as usize / 2, "keyed by the run, a range for each hour that appended: {run_held_first}");
    assert!(run_held_second > run_held_first + BLOCK as usize / 2, "and it grows with the run: {run_held_first} then {run_held_second}");
    assert_eq!([hourly_held_first, hourly_held_second, hourly_saved_first, hourly_saved_second], [0; 4], "keyed by its hour, no key held or saved past its hour");
    for (run, hourly, ranges) in [(&run_first, &hourly_first, run_held_first), (&run_second, &hourly_second, run_held_second)] {
        let mut count = Bytes::new();
        count.varint(ranges as u64).unwrap();
        assert_eq!(run.len() - hourly.len(), count.bytes().len() + 32 * ranges, "the state keyed by the run is its ranges more, and nothing else");
    }
}

#[derive(Clone, Copy, Debug)]
enum Misuse {
    SetLog,
    DeleteLog,
    AppendState,
    ForgetState,
    KeyAgain,
    KeyAgainInHour,
    KeyOfAnotherHour,
    ForgetNever,
    ForgetAgain,
}

struct Misusing {
    misuse: Misuse,
}

impl Writer for Misusing {
    fn name(&self) -> &'static str {
        "Misusing"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        match self.misuse {
            Misuse::KeyAgainInHour | Misuse::KeyOfAnotherHour => &[Output::Log(TOY, Keys::Hour(32)), Output::State(TableId::Campaign)],
            _ => &[Output::Log(TOY, Keys::Run), Output::State(TableId::Campaign)],
        }
    }

    fn reads(&self) -> &'static [Read] {
        &[]
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {}

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()> {
        let cells = |seed: u64, out: &mut Rows<'_>| vec![Value::U64(seed), Value::Bytes(0), out.opaque(vec![1]), Value::Unrecorded];
        if hour.hour == 0 {
            for key in [1, 2] {
                let row = cells(key, out);
                out.append(TOY, key as u128, row);
            }
            out.forget(TOY, 1);
            out.set(TableId::Campaign, 1, vec![Value::Bytes(0)]);
            return Ok(());
        }
        let row = cells(9, out);
        let key = u128::from(hour.hour as u32) << 32 | 9;
        out.append(TOY, key, row);
        match self.misuse {
            Misuse::SetLog => {
                let row = cells(3, out);
                out.set(TOY, 3, row);
            }
            Misuse::DeleteLog => out.delete(TOY, 1),
            Misuse::AppendState => {
                out.append(TableId::Campaign, 3, vec![Value::Bytes(0)]);
            }
            Misuse::ForgetState => out.forget(TableId::Campaign, 0),
            Misuse::KeyAgain | Misuse::KeyOfAnotherHour => {
                let row = cells(1, out);
                out.append(TOY, 1, row);
            }
            Misuse::KeyAgainInHour => {
                let row = cells(10, out);
                out.append(TOY, key, row);
            }
            Misuse::ForgetNever => out.forget_run(TOY, 2, 4),
            Misuse::ForgetAgain => out.forget(TOY, 1),
        }
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_writer_misusing_its_log_or_its_state_table_is_withheld_by_name() {
    for (misuse, refusal) in [
        (Misuse::SetLog, "it set a row of log table NuclearStrikes: a log's rows are appended and forgotten"),
        (Misuse::DeleteLog, "it deleted a row of log table NuclearStrikes: a log's rows are appended and forgotten"),
        (Misuse::AppendState, "it appended to state table Campaign: a state table's rows are set and deleted"),
        (Misuse::ForgetState, "it forgot rows of state table Campaign: a state table's rows are set and deleted"),
        (Misuse::KeyAgain, "it gave log table NuclearStrikes key 0x1 again: a log holds a key once a run"),
        (Misuse::KeyAgainInHour, "it gave log table NuclearStrikes key 0x100000009 again: a log holds a key once a run"),
        (Misuse::KeyOfAnotherHour, "it gave log table NuclearStrikes key 0x1 of hour 0 in hour 1: its keys carry the hour they are appended in"),
        (Misuse::ForgetNever, "it forgot NuclearStrikes row 3, which the log never appended"),
        (Misuse::ForgetAgain, "it forgot NuclearStrikes row 1, which the log forgot before"),
    ] {
        let mut store = toy_store(false);
        let mut writers = Writers::new(vec![Box::new(Misusing { misuse })]);
        writers.start_run();
        store.begin_run(0);
        let listed = |store: &Store| {
            let held = store.table_state(TOY.index()).unwrap();
            (held.count(), held.listed().unwrap().ones().collect::<Vec<_>>())
        };
        for hour in 0..3 {
            let appends: &[&[u8]] = if hour == 0 { &[b"a"] } else { &[] };
            store.begin_hour(hour, appends).unwrap();
            writers.hour(1, hour, hour == 0, &mut store).unwrap_or_else(|error| panic!("{misuse:?}: the recording goes on: {error}"));
            store.end_hour(&mut replay_columnar::unit::Discard).unwrap();
            if hour == 0 {
                assert_eq!(listed(&store), (Some(2), vec![0]), "{misuse:?}");
                continue;
            }
            for table in [TOY, TableId::Campaign] {
                assert_eq!(reason(&store, table.index()), format!("the recorder stopped working this out: {refusal}"), "{misuse:?}: {table:?} at hour {hour}");
            }
            assert_eq!(listed(&store), (Some(2), vec![0]), "{misuse:?}: nothing of the failed hour landed");
            assert_eq!(rows_of(&store, TableId::Campaign), [(1, Some(b"a".to_vec()))], "{misuse:?}");
        }
    }
}

struct Declared {
    tables: &'static [Output],
}

impl Writer for Declared {
    fn name(&self) -> &'static str {
        "Declared"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        self.tables
    }

    fn reads(&self) -> &'static [Read] {
        &[]
    }

    fn start(&mut self) {}

    fn hour(&mut self, _: &Hour<'_>, _: &mut Rows<'_>) -> io::Result<()> {
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_writer_declaring_a_table_s_class_otherwise_than_the_schema_or_the_store_is_refused_by_name() {
    const CAMPAIGN_LOG: &[Output] = &[Output::Log(TableId::Campaign, Keys::Run)];
    const TOY_STATE: &[Output] = &[Output::State(TOY)];
    const TOY_LOG: &[Output] = &[Output::Log(TOY, Keys::Run)];
    const TOY_HOUR_PAST: &[Output] = &[Output::Log(TOY, Keys::Hour(97))];
    for (tables, store, refusal) in [
        (CAMPAIGN_LOG, toy_store(false), "Declared declares Campaign a log, which the schema holds as state"),
        (TOY_STATE, toy_store(false), "Declared declares NuclearStrikes state, which the schema holds as a log"),
        (TOY_LOG, store(), "Declared declares NuclearStrikes a log, which the store holds as state"),
        (TOY_HOUR_PAST, toy_store(false), "Declared declares the keys of NuclearStrikes carry their hour at bit 97: a key's 128 bits end before the hour's 32"),
    ] {
        let mut store = store;
        let mut writers = Writers::new(vec![Box::new(Declared { tables })]);
        writers.start_run();
        store.begin_run(0);
        store.begin_hour(0, &[]).unwrap();
        let refused = writers.hour(1, 0, true, &mut store).unwrap_err();
        assert_eq!((refused.kind(), refused.to_string()), (io::ErrorKind::InvalidInput, refusal.to_owned()));
        assert_eq!(store.view(TOY.index()).status(), Status::NotRecorded, "no writer ran");
    }
}
