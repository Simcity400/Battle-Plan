use super::*;
use crate::identity::TagHolders;
use replay_data::rows::TABLE_COUNT;
use std::sync::Arc;

/// A schema of every table, those the tests use with their real kinds, the rest empty.
fn schema() -> Vec<Vec<Kind>> {
    let mut schema = vec![Vec::new(); TABLE_COUNT];
    schema[TableId::Countries.index()] = vec![Kind::U32, Kind::Bytes];
    schema[TableId::CountryLinks.index()] = vec![Kind::U32];
    schema[TableId::TagHolders.index()] = vec![Kind::Bytes];
    schema[TableId::TagNames.index()] = vec![Kind::Bytes];
    schema[TableId::Campaign.index()] = vec![Kind::Bytes];
    schema[TableId::CountryStability.index()] = vec![Kind::I64, Kind::I64, Kind::I64];
    schema[LOG.index()] = vec![Kind::U64, Kind::I32];
    schema[ORDERED.index()] = vec![Kind::U64, Kind::OptionalI64, Kind::Bytes, Kind::Opaque];
    let [by_key, by_ordinal] = EVENTS_TABLES;
    schema[by_key.index()] = vec![Kind::U64, Kind::U64];
    schema[by_ordinal.index()] = vec![Kind::U64];
    schema[TOY.index()] = TOY_KINDS.to_vec();
    schema
}

fn store() -> Store {
    Store::new(schema(), vec![false; TABLE_COUNT])
}

/// A writer of Campaign that fails at hour `fails`.
struct Failing {
    fails: i32,
}

impl Writer for Failing {
    fn name(&self) -> &'static str {
        "Failing"
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

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        if hour.hour >= self.fails {
            return Err(invalid("it broke"));
        }
        out.set(TableId::Campaign, 0, vec![Value::Bytes(0)]);
        Ok(())
    }

    fn save(&self, _: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

/// Hour `hour` of Countries (id, slot, tag id) and CountryLinks (id, slot), complete or
/// `status`, applied to `store`: the tables replaced whole.
fn apply(store: &mut Store, hour: i32, countries: &[(u32, u32, u32)], links: &[(u32, u32)], status: Status) {
    let appends: &[&[u8]] = if hour == 0 { &[b"GER", b"ENG"] } else { &[] };
    apply_with(store, hour, appends, countries, links, status);
}

fn apply_with(store: &mut Store, hour: i32, appends: &[&[u8]], countries: &[(u32, u32, u32)], links: &[(u32, u32)], status: Status) {
    store.begin_hour(hour, appends).unwrap();
    for table in [TableId::Countries, TableId::CountryLinks] {
        let index = table.index();
        let held = if table == TableId::Countries { status } else { Status::Complete };
        store.status(index, held, true, Some(hour), true).unwrap();
        let live: Vec<u32> = store.view(index).rows().collect();
        let mut changes = store.table(index).unwrap();
        for slot in live {
            changes.delete(slot).unwrap();
        }
        match table {
            TableId::Countries => {
                for &(id, slot, tag) in countries {
                    let at = changes.insert(u128::from(id)).unwrap();
                    changes.write(at, 0, Cell::U32(slot)).unwrap();
                    changes.write(at, 1, Cell::Bytes(tag)).unwrap();
                }
            }
            _ => {
                for &(id, slot) in links {
                    let at = changes.insert(u128::from(id)).unwrap();
                    changes.write(at, 0, Cell::U32(slot)).unwrap();
                }
            }
        }
    }
    store.settle();
}

/// `tag`'s live holder row: its holder, if resolved, from the row's key.
fn holder(store: &Store, tag: &[u8]) -> Option<Option<(u32, u32)>> {
    let table = store.view(TableId::TagHolders.index());
    table.rows().find_map(|slot| {
        let id = table.text(0, slot)?;
        (table.string(id) == Some(tag)).then(|| {
            let key = table.key(slot).unwrap();
            let (slot, id) = ((key >> 32) as u32, key as u32);
            (slot != u32::MAX).then_some((slot, id))
        })
    })
}

fn reason(store: &Store, table: usize) -> String {
    match store.view(table).status() {
        Status::Unavailable { reason } => String::from_utf8_lossy(store.strings().get(reason).unwrap()).into_owned(),
        other => panic!("{other:?}"),
    }
}

/// The first holder by id whose link resolves holds its tag; a tag no link resolves has none; a
/// writer that fails withholds its own table by name for the rest of the run and nothing else; an
/// input not complete closes every holder row and opens an unknown one, naming why; a resume
/// takes every writer up where it was.
#[test]
fn writers_run_withhold_and_resume_by_their_own_rules() {
    let mut store = store();
    // The failing writer's own reason, in words; never its name.
    const BROKE: &str = "the recorder stopped working this out: it broke";
    let mut writers = Writers::new(vec![Box::new(TagHolders::default()), Box::new(Failing { fails: 2 })]);
    writers.start_run();
    let holders = TableId::TagHolders.index();
    let campaign = TableId::Campaign.index();
    // Hour 0: GER is countries 7 (unresolved) and 9 (resolved); ENG is 4, unresolved.
    apply(&mut store, 0, &[(9, 3, 0), (7, 3, 0), (4, 5, 1)], &[(9, 3), (7, 8), (4, 6)], Status::Complete);
    writers.hour(1, 0, true, &mut store).unwrap();
    assert_eq!(store.view(holders).status(), Status::Complete);
    assert_eq!(holder(&store, b"GER"), Some(Some((3, 9))));
    assert_eq!(holder(&store, b"ENG"), Some(None));
    assert_eq!(store.view(campaign).status(), Status::Complete);
    // Hour 1: ENG's link resolves.
    apply(&mut store, 1, &[(9, 3, 0), (7, 3, 0), (4, 5, 1)], &[(9, 3), (7, 8), (4, 5)], Status::Complete);
    writers.hour(1, 1, false, &mut store).unwrap();
    assert_eq!(holder(&store, b"ENG"), Some(Some((5, 4))));
    // Hour 2: the other writer fails; TagHolders goes on.
    apply(&mut store, 2, &[(9, 3, 0), (4, 5, 1)], &[(9, 3), (4, 5)], Status::Complete);
    writers.hour(1, 2, false, &mut store).unwrap();
    assert_eq!(reason(&store, campaign), BROKE);
    assert_eq!(store.view(holders).status(), Status::Complete);
    // Hour 3: Countries unavailable; no holder is known, and the unknown row says why.
    apply(&mut store, 3, &[(9, 3, 0), (4, 5, 1)], &[(9, 3), (4, 5)], Status::Unavailable { reason: 0 });
    writers.hour(1, 3, false, &mut store).unwrap();
    assert_eq!(store.view(holders).status(), Status::Complete);
    assert_eq!(holder(&store, b"GER"), None);
    let table = store.view(holders);
    let unknown: Vec<u128> = table.rows().filter_map(|slot| table.key(slot)).filter(|key| (key >> 96) as u32 == u32::MAX).collect();
    assert_eq!(unknown.len(), 1, "one unknown row lives, naming Countries");
    assert_eq!((unknown[0] >> 32) as u32, 0);
    assert_eq!(reason(&store, campaign), BROKE);
    // A resume takes the writers up: the failed one stays withheld.
    let saved = writers.save().unwrap();
    let mut resumed = Writers::new(vec![Box::new(TagHolders::default()), Box::new(Failing { fails: 99 })]);
    resumed.load(&saved, &mut store).unwrap();
    apply(&mut store, 4, &[(9, 3, 0)], &[(9, 3)], Status::Complete);
    resumed.hour(1, 4, false, &mut store).unwrap();
    assert_eq!(reason(&store, campaign), BROKE);
    assert_eq!(store.view(holders).status(), Status::Complete);
    assert_eq!(holder(&store, b"ENG"), None);
    assert_eq!(holder(&store, b"GER"), Some(Some((3, 9))));
    // A writer of another version is withheld by name, taken up as a resume is: the store's run
    // begun again.
    store.begin_run(5);
    let mut other = Writers::new(vec![Box::new(TagHolders::default())]);
    let mut blob = Bytes::new();
    blob.varint(u64::from(DERIVED_STRINGS)).unwrap();
    blob.varint(0).unwrap();
    blob.varint(1).unwrap();
    blob.varint(10).unwrap();
    blob.raw(b"TagHolders").unwrap();
    blob.varint(99).unwrap();
    blob.u8(1).unwrap();
    blob.u8(0).unwrap();
    blob.varint(0).unwrap();
    blob.varint(0).unwrap();
    other.load(blob.bytes(), &mut store).unwrap();
    apply_with(&mut store, 5, &[b"GER", b"ENG"], &[(9, 3, 0)], &[(9, 3)], Status::Complete);
    other.hour(1, 5, false, &mut store).unwrap();
    assert_eq!(reason(&store, holders), OTHER_VERSION, "never the writer's name");
}

/// `table`'s rows: each key with its first cell's text.
fn rows_of(store: &Store, table: TableId) -> Vec<(u128, Option<Vec<u8>>)> {
    let view = store.view(table.index());
    let mut rows: Vec<_> = view
        .rows()
        .map(|slot| (view.key(slot).unwrap(), view.text(0, slot).and_then(|id| view.string(id)).map(<[u8]>::to_vec)))
        .collect();
    rows.sort_unstable();
    rows
}

/// TagHolders numbers its rows (each key's epoch) in tag order: two writers given the same hours
/// in one process, their countries inserted in other orders, write the same rows. A writer
/// numbering by a hash map's order, seeded per map, writes them numbered otherwise.
#[test]
fn tag_holders_number_rows_alike_whatever_order_their_countries_came_in() {
    const TAGS: u32 = 64;
    let names: Vec<Vec<u8>> = (0..TAGS).map(|tag| format!("T{tag:02}").into_bytes()).collect();
    let appends: Vec<&[u8]> = names.iter().map(Vec::as_slice).collect();
    // Tag t's countries are 2t (its link unresolved until hour 1 for even t) and 2t + 1.
    let countries: Vec<(u32, u32, u32)> = (0..TAGS).flat_map(|tag| [(2 * tag, 100 + tag, tag), (2 * tag + 1, 100 + tag, tag)]).collect();
    let links = |hour: i32| -> Vec<(u32, u32)> {
        (0..TAGS)
            .flat_map(|tag| {
                let resolves = hour > 0 && tag % 2 == 0;
                [(2 * tag, if resolves { 100 + tag } else { 999 }), (2 * tag + 1, 100 + tag)]
            })
            .collect()
    };
    let orders: [Box<dyn Fn(&mut Vec<(u32, u32, u32)>, &mut Vec<(u32, u32)>)>; 2] = [
        Box::new(|_, _| {}),
        Box::new(|countries, links| {
            countries.reverse();
            countries.rotate_left(17);
            links.reverse();
            links.rotate_left(29);
        }),
    ];
    let mut written = Vec::new();
    for order in &orders {
        let mut store = store();
        let mut writers = Writers::new(vec![Box::new(TagHolders::default())]);
        writers.start_run();
        for hour in 0..2 {
            let (mut countries, mut links) = (countries.clone(), links(hour));
            order(&mut countries, &mut links);
            apply_with(&mut store, hour, if hour == 0 { &appends } else { &[] }, &countries, &links, Status::Complete);
            writers.hour(1, hour, hour == 0, &mut store).unwrap();
        }
        written.push((rows_of(&store, TableId::TagHolders), rows_of(&store, TableId::TagNames)));
    }
    assert_eq!(written[0].0.len(), TAGS as usize);
    // Hour 1 gave every even tag a new holder: a new row, numbered after hour 0's.
    let epochs: Vec<u32> = written[0].0.iter().map(|(key, _)| (key >> 64) as u32).collect();
    assert_eq!(epochs.iter().filter(|&&epoch| epoch >= TAGS).count(), TAGS as usize / 2);
    assert_eq!(written[0], written[1], "the same hours number the same rows alike");
}

/// A writer setting `rows` of Campaign every hour.
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

/// A writer setting rows equal to those its table holds writes nothing: no insert, no written
/// cell in the hour's log; a cell set differently is written alone, and one set unrecorded is
/// unrecorded.
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

/// Memory labels, in a build counting them.
#[cfg(feature = "memory-labels")]
mod labelled {
    use super::*;

    /// A writer keeping `bytes` more of its own each hour and setting `rows` rows of Campaign.
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

    /// What a writer keeps from its hour is charged to its own label, and what its rows add to the
    /// store (here tens of thousands of rows) and the hour's rows it handed over are not.
    #[test]
    fn what_a_writer_allocates_in_its_hour_is_charged_to_its_own_label_and_its_tables_growth_to_the_store() {
        const KEPT: i64 = 4 << 20;
        let mut store = store();
        store.begin_hour(0, &[b"a"]).unwrap();
        let mut writers = Writers::new(vec![Box::new(Hoarding { bytes: KEPT as usize, rows: 40_000, held: Vec::new() })]);
        writers.start_run();
        writers.hour(1, 0, true, &mut store).unwrap();
        assert_eq!(store.view(TableId::Campaign.index()).rows().count(), 40_000);
        let phases = labels::phases();
        let held = |owned: &dyn Fn(&str) -> bool| -> i64 {
            phases.iter().flat_map(|phase| &phase.labels).filter(|charge| owned(charge.owner) && charge.kind == labels::Kind::Operator).map(|charge| charge.end).sum()
        };
        let writer = held(&|owner| owner == "writer: Hoarding");
        assert!((KEPT..KEPT + (64 << 10)).contains(&writer), "the writer's label holds {writer} bytes");
        // The store names its growth by part: "store: cells grown", "store: index grown", ...
        let store = held(&|owner| owner == "store" || owner.starts_with("store: "));
        assert!(store >= 40_000 * 4, "the store's labels hold {store} bytes");
    }
}

/// A table the hour landed (here its status told) before the writer of it ran is refused by name:
/// a table has one source, so an hour landing a writer's table stops the run at its first hour.
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

/// A writer of Campaign echoing what the hour says it changed of Countries and CountryLinks: a
/// row per country the hour wrote, and one when it wrote a link.
struct Echo;

impl Writer for Echo {
    fn name(&self) -> &'static str {
        "Echo"
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

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let countries = hour.table(TableId::Countries);
        for slot in hour.written_rows(TableId::Countries, Columns::Every) {
            let tag = countries.text(1, slot).unwrap_or(0);
            out.set(TableId::Campaign, countries.key(slot).unwrap_or(0), vec![Value::Bytes(tag)]);
        }
        if hour.wrote(TableId::CountryLinks, Columns::Every) && hour.changed(TableId::Countries) {
            out.set(TableId::Campaign, u128::MAX, vec![Value::Bytes(0)]);
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

/// A run's first hour, which keeps no hour log, gives the writers what the same hour logged gives
/// them: the same rows of every table they write.
#[test]
fn a_run_s_first_hour_without_its_log_gives_the_writers_what_it_logged_does() {
    let countries = [(9, 3, 0), (7, 3, 0), (4, 5, 1)];
    let links = [(9, 3), (7, 8), (4, 6)];
    let mut written = Vec::new();
    for logged in [false, true] {
        let mut store = store();
        let mut writers = Writers::new(vec![Box::new(TagHolders::default()), Box::new(Echo)]);
        writers.start_run();
        if logged {
            store.begin_run(-1);
            store.begin_hour(-1, &[b"GER", b"ENG"]).unwrap();
            store.settle();
            apply_with(&mut store, 0, &[], &countries, &links, Status::Complete);
            assert!(!store.log().tables[TableId::Countries.index()].inserted.is_empty());
        } else {
            store.begin_run(0);
            apply(&mut store, 0, &countries, &links, Status::Complete);
            assert!(store.log().tables[TableId::Countries.index()].whole);
        }
        writers.hour(1, 0, true, &mut store).unwrap();
        written.push([TableId::TagHolders, TableId::TagNames, TableId::Campaign].map(|table| rows_of(&store, table)));
    }
    assert_eq!(written[0][2].len(), 4, "a row per country and one for the links");
    assert_eq!(written[0], written[1]);
}

/// The log the log tests read: each row a value and the hour it came.
const LOG: TableId = TableId::MarketEvents;
/// The tables of the log's two readers: by the row's key its ordinal and value, and by ordinal
/// the row's key. No other test here may use these four: [`schema`] gives them the readers' kinds.
const EVENTS_TABLES: [TableId; 2] = [TableId::ArmyTotals, TableId::ArmyMix];
const EVENTS_OUTPUTS: [Output; 2] = [Output::State(EVENTS_TABLES[0]), Output::State(EVENTS_TABLES[1])];
const EVENTS: &[Read] = &[Read { table: LOG, columns: Columns::Every, watch: Watch::Appended, required: false }];

/// A store whose MarketEvents is a log.
fn log_store() -> Store {
    let mut store = store();
    let mut logs = vec![false; TABLE_COUNT];
    logs[LOG.index()] = true;
    store.declare_logs(&logs).unwrap();
    store
}

/// What a log's reader was handed in an hour it ran.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Handed {
    hour: i32,
    /// Each appended row's ordinal, key and value.
    appended: Vec<(u64, u128, u64)>,
    backlog: bool,
    forgotten: Vec<std::ops::Range<u64>>,
}

type Handings = std::sync::Arc<std::sync::Mutex<Vec<Handed>>>;

/// A writer following the log: every row it is handed by its key (its ordinal and value) and by its ordinal (its key), a row forgotten leaving both unless its key
/// came again; all it follows is in its tables.
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

/// Lands hour `hour` of the log: `appends` (key, value) in order, then `forgets`, and its status
/// told.
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

/// The keys `table` holds, ascending.
fn keys_of(store: &Store, table: TableId) -> Vec<u128> {
    let view = store.view(table.index());
    let mut keys: Vec<u128> = view.rows().filter_map(|slot| view.key(slot)).collect();
    keys.sort_unstable();
    keys
}

fn follow(name: &'static str, tables: &'static [Output; 2], reads: &'static [Read], handed: &Handings) -> Box<dyn Writer> {
    Box::new(Follow { name, tables, reads, handed: handed.clone() })
}

/// A writer reading a log is handed, in each hour the log appended or forgot rows or its status
/// changed, the appended rows with their ordinals, keys and cells and the forgotten ordinals: in
/// a run's first hour the list as it stood, flagged backlog, and never after. A log not complete
/// makes its readers' tables what it is, its reason kept.
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
    // Nothing appended, forgotten or told otherwise: not run.
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

/// A row the game changes in place comes as a forget of its old ordinal and an append of its key
/// in one hour: both are handed, a writer keyed by the row's key holds the new cells once and the
/// old ordinal is gone from its own tables, and a writer counting appends sees one new row.
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

/// A writer reading a log with another watch than Appended, or reading a state table with it, is
/// refused by name at its run's first hour, before any writer runs.
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

/// Bytes the test's thread holds from the allocator, and the most it held since last asked; in a
/// build counting memory labels, over the labels' allocator.
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

    /// The most bytes `run` held at once on this thread beyond what the thread held before it.
    pub fn peak(run: impl FnOnce()) -> usize {
        let before = HELD.with(Cell::get);
        PEAK.with(|peak| peak.set(before));
        run();
        (PEAK.with(Cell::get) - before).max(0) as usize
    }
}

#[global_allocator]
static COUNTING: counting::Counting = counting::Counting;

/// A writer setting every row it holds again, each equal to the one held (as a writer reading its
/// inputs whole after a gap does), holds no copy of what it sets: its hour holds under four bytes
/// a row at once. A framework holding every row set until the writer returns holds some sixty.
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

/// A writer of Campaign: rows 1 and 3 until hour `fails`, where it changes row 1, adds row 2,
/// deletes row 3 and then fails.
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

/// A writer failing after it set rows leaves its table as the hour before left it, unavailable
/// for its reason: no row the failed hour set, changed or deleted is read, the hour logs none, and
/// the text it named is no string of the run.
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

/// A writer of Campaign that, after its first hour, sets row 2 twice or reads its own table
/// through the hour.
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

/// A writer setting one row twice in an hour, or reading its own table through the hour rather
/// than through its rows (where the hour shows no row of it), fails that hour by name: its table
/// unavailable for the reason, the hour's rows undone.
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

/// The table of the order tests: a number, an optional, a text and an opaque cell.
const ORDERED: TableId = TableId::WarEvents;

/// A writer of [`ORDERED`] setting each hour the same rows, cells and deletes in `order`: rows
/// leave and come back, cells change, go missing and are unrecorded, and a row deleted and set in
/// one hour is kept.
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

/// A unit written to memory.
#[derive(Default)]
struct Memory {
    bytes: Vec<u8>,
    pages: Vec<replay_columnar::unit::BlockPage>,
}

impl replay_columnar::unit::PartSink for Memory {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64> {
        let at = self.bytes.len() as u64;
        self.bytes.extend_from_slice(stored);
        Ok(at)
    }

    fn block(&mut self, page: replay_columnar::unit::BlockPage) -> io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn read_back(&mut self, at: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.extend_from_slice(&self.bytes[at as usize..at as usize + stored as usize]);
        Ok(())
    }
}

/// What a run of [`Ordered`] setting its rows in `order` writes: the unit, its block pages, and
/// the key at each slot of its table.
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

/// A writer setting and deleting its rows in any order writes the unit, byte for byte, and the
/// slots of one doing so in key order, which is what a framework applying each hour's rows sorted
/// by key wrote: new rows take the slots key order gives them and changed cells reach the unit in
/// key order.
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

/// Each hour a writer watching Countries every read was run, and whether the hour it was handed
/// says Countries was read then (its sample hour is the hour).
type Ran = std::sync::Arc<std::sync::Mutex<Vec<(i32, bool)>>>;

const EVERY_COUNTRIES: &[Read] = &[Read { table: TableId::Countries, columns: Columns::Every, watch: Watch::EveryRead, required: false }];

/// A writer of `table` watching Countries every read, run only when it is due by its reads, or
/// every hour when `hourly`.
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

/// An hour of no section through the journal door: Countries and CountryLinks complete, as hour 0
/// left them, Countries' read bit as `read` says.
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

/// Countries read every read moves the hour exactly when the journal stamps it read, though its
/// rows equal the hour before's: a writer the framework runs by its reads runs then and only
/// then. An hourly writer runs every hour, and the hour it is handed says Countries was
/// not read in the hour the journal did not stamp it: what a writer counting each read (the
/// equipment-loss drain) goes by.
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

/// A writer of the supply thresholds, as supply health declares them, into CountryStability's
/// row 0 (grace, low, the two icon thresholds summed), noting what else it was handed.
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
    let needs = installed_content_core::unit_leader::needs::SubUnitNeeds {
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

/// A writer declaring definitions it requires is not run while they are absent: its table is
/// unavailable by name and holds no row, nothing written as zero. Once they are held its table is
/// right from that hour on, read fresh; a resume hands them again to `load` and `prime`, and the
/// writer goes on as one that never stopped. The supply thresholds and a sub-unit's need reach
/// it by name.
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
    // The same definitions again change nothing: the writer is not made fresh.
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

/// Definitions read from a pack are only the kinds the registered writers declare, through the
/// reader the writers hold; nothing is read when no writer declares any.
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
    let mut none = Writers::new(vec![Box::new(TagHolders::default())]);
    none.read_definitions_with(|_, _, _| panic!("nothing is read when no writer declares a definition"));
    none.define_from(&Facts::held(Arc::new(b"pack".to_vec())), &identity());
}

/// The hours a writer was called, each with whether it read its inputs whole.
type Calls = std::sync::Arc<std::sync::Mutex<Vec<(i32, bool)>>>;

/// A writer of Campaign reading Countries (required) by its changes, telling its table read in
/// part each hour it is called and setting nothing else; `hourly` when it declares that duty.
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

/// A writer withheld for an input not complete says why in words a user reads: what the input
/// holds and the input's own reason, never the writer's or the table's identifier.
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

/// The hours [`Called`] is called over seven: Countries written at hours 0, 2 and 5, not complete
/// at 4, untouched at 1, 3 and 6; its table's status and sample hour after each hour.
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

/// A writer reading state tables, with no hourly duty, is not called in an hour that changed none
/// of its inputs: it is called in its run's first hour, in an hour that wrote its input, and in the
/// hour after one it did not run, reading its inputs whole. Its table in an hour it is not called
/// keeps the status it last told, read that hour. A framework calling every writer reading no log
/// each hour calls it at 1, 3 and 6 too.
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

/// A writer declaring an hourly duty is called every hour its inputs are complete, whatever they
/// did.
#[test]
fn an_hourly_writer_is_called_every_hour() {
    let (calls, _) = called(true);
    assert_eq!(calls, [(0, true), (1, false), (2, false), (3, false), (5, true), (6, false)]);
}

/// A lane writer called by its own declaration, or (`every`) each hour, counting its calls.
struct Counted {
    inner: Box<dyn Writer>,
    every: bool,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Writer for Counted {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn version(&self) -> u32 {
        self.inner.version()
    }

    fn tables(&self) -> &'static [Output] {
        self.inner.tables()
    }

    fn reads(&self) -> &'static [Read] {
        self.inner.reads()
    }

    fn hourly(&self) -> bool {
        self.every || self.inner.hourly()
    }

    fn strings(&self) -> &'static [&'static str] {
        self.inner.strings()
    }

    fn definitions(&self) -> &'static [Uses] {
        self.inner.definitions()
    }

    fn start(&mut self) {
        self.inner.start();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.inner.hour(hour, out)
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        self.inner.save(blob)
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        self.inner.load(blob)
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        self.inner.prime(hour);
    }
}

/// A number cell of `kind` holding `value`; of Bytes, string `value`.
fn number(kind: Kind, value: i64) -> Cell<'static> {
    match kind {
        Kind::U8 => Cell::U8(value as u8),
        Kind::I8 => Cell::I8(value as i8),
        Kind::U32 => Cell::U32(value as u32),
        Kind::I32 => Cell::I32(value as i32),
        Kind::U64 => Cell::U64(value as u64),
        Kind::I64 => Cell::I64(value),
        Kind::OptionalI64 => Cell::OptionalI64(Some(value)),
        Kind::Bytes => Cell::Bytes(value as u32),
        other => panic!("no number cell of {other:?}"),
    }
}

/// One table's rows of an hour: each key with its cells by column.
type Given = (TableId, Vec<(u128, Vec<(usize, i64)>)>);

/// Hour `hour` of a short campaign of three countries, as the lane's writers read it: each table
/// told complete and its rows offered, a cell written only where it moved. Most hours move one
/// table, some none: political power at every third hour, hit points at 4, 5 and 18, a state's
/// controller at 7, a link unresolved from 10 to 13, victory points at 12, a war ending at 20.
fn campaign_hour(store: &mut Store, hour: i32) {
    use replay_data::rows::{CountryLinkRow, CountryRow, DivisionRow, HumanControllerRow, ProvinceRow, StateRow, WarRelationRow};
    let appends: &[&[u8]] = if hour == 0 { &[b"GER", b"ENG", b"FRA"] } else { &[] };
    store.begin_hour(hour, appends).unwrap();
    let country = |id: i64| (id - 1) % 3 + 1;
    let from = |first: i32| i64::from(hour >= first);
    let tables: [Given; 7] = [
        (
            TableId::Countries,
            (1..=3)
                .map(|id: i64| {
                    let political = if id == 1 { 100 - 5 * i64::from(hour / 3) } else { 50 };
                    let cells = [("canonical_slot", id), ("tag", id - 1), ("political_power_raw", political), ("command_power_raw", 20)];
                    (id as u128, cells.map(|(name, value)| (column::<CountryRow>(name), value)).to_vec())
                })
                .collect(),
        ),
        (
            TableId::CountryLinks,
            (1..=3)
                .map(|id: i64| {
                    let slot = if id == 3 && (10..14).contains(&hour) { 9 } else { id };
                    (id as u128, vec![(column::<CountryLinkRow>("canonical_slot"), slot)])
                })
                .collect(),
        ),
        (
            TableId::States,
            (1..=4)
                .map(|id: i64| {
                    let controller = if id == 2 && hour >= 7 { 1 } else { country(id) };
                    (id as u128, vec![(column::<StateRow>("owner"), country(id)), (column::<StateRow>("controller"), controller)])
                })
                .collect(),
        ),
        (
            TableId::Provinces,
            (1..=6)
                .map(|id: i64| {
                    let points = id + if id == 5 { 10 * from(12) } else { 0 };
                    let cells = [("owner", country(id)), ("controller", country(id)), ("victory_points", points)];
                    (id as u128, cells.map(|(name, value)| (column::<ProvinceRow>(name), value)).to_vec())
                })
                .collect(),
        ),
        (
            TableId::Divisions,
            (1..=5)
                .map(|id: i64| {
                    let points = 300_000 - 1_000 * (from(4) + from(5) + from(18)) * i64::from(id == 2);
                    let cells = [("roster_owner", country(id)), ("expeditionary_owner", 0), ("template", 7), ("hit_points", points)];
                    (id as u128, cells.map(|(name, value)| (column::<DivisionRow>(name), value)).to_vec())
                })
                .collect(),
        ),
        (TableId::HumanControllers, vec![(1, vec![(column::<HumanControllerRow>("count"), 1)])]),
        (
            TableId::WarRelations,
            vec![(1 << 32 | 2, [("start_hour", 0), ("end_hour", 20), ("first_was_instigator", 1)].map(|(name, value)| (column::<WarRelationRow>(name), value)).to_vec())],
        ),
    ];
    for (table, rows) in tables {
        let index = table.index();
        store.status(index, Status::Complete, true, Some(hour), true).unwrap();
        let kinds = store.kinds(index).to_vec();
        let mut changes = store.table(index).unwrap();
        for (key, cells) in rows {
            match changes.slot(key) {
                Some(slot) => {
                    for (column, value) in cells {
                        changes.offer(slot, column, number(kinds[column], value)).unwrap();
                    }
                }
                None => {
                    let slot = changes.insert(key).unwrap();
                    for (column, value) in cells {
                        changes.write(slot, column, number(kinds[column], value)).unwrap();
                    }
                }
            }
        }
    }
    store.settle();
}

/// What a run of the lane's writers wrote: the unit and its block pages.
type Unit = (Vec<u8>, Vec<replay_columnar::unit::BlockPage>);

/// What the lane's writers write over 24 hours of [`campaign_hour`], each
/// called by its declaration or, `every`, each hour: the unit, how many calls they took, and how
/// many rows TagHolders, ArmyTotals and TerritoryTotals hold at the end.
fn lane_unit(every: bool) -> (Unit, usize, [usize; 3]) {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let lane = crate::writers()
        .writers
        .into_iter()
        .map(|inner| Box::new(Counted { inner, every, calls: calls.clone() }) as Box<dyn Writer>)
        .collect();
    let kinds = crate::testing::kinds();
    let mut store = Store::new(kinds.clone(), vec![false; TABLE_COUNT]);
    store.declare_logs(&replay_data::rows::logs()).unwrap();
    store.attach(replay_columnar::unit::Encoder::new(&kinds, replay_columnar::unit::Settings::at(3)).unwrap());
    let mut writers = Writers::new(lane);
    writers.start_run();
    let mut unit = Memory::default();
    store.begin_run(0);
    for hour in 0..24 {
        campaign_hour(&mut store, hour);
        writers.hour(1, hour, hour == 0, &mut store).unwrap();
        store.end_hour(&mut unit).unwrap();
    }
    store.end_run(&mut unit).unwrap();
    let held = [TableId::TagHolders, TableId::ArmyTotals, TableId::TerritoryTotals].map(|table| store.view(table.index()).rows().count());
    ((unit.bytes, unit.pages), calls.load(std::sync::atomic::Ordering::Relaxed), held)
}

/// The lane's writers, each not called in an hour that moved none of its inputs unless it declares
/// an hourly duty, write the unit byte for byte as when every one is called each hour: a writer
/// not called is one that would have set every row equal to the one held, and its statuses so.
#[test]
fn the_lane_s_writers_write_the_bytes_they_wrote_when_called_every_hour() {
    let (gated, gated_calls, held) = lane_unit(false);
    let (every, every_calls, _) = lane_unit(true);
    assert!(held.iter().all(|&rows| rows > 0), "TagHolders, ArmyTotals and TerritoryTotals hold rows: {held:?}");
    assert!(gated_calls < every_calls, "{gated_calls} calls gated against {every_calls} every hour");
    assert!(gated.0 == every.0, "the unit's bytes differ");
    assert_eq!(gated.1, every.1);
}

/// The texts every synthetic Bytes cell names, appended at the run's first hour.
const TEXTS: [&[u8]; 3] = [b"GER", b"ENG", b"FRA"];

/// A field of `kind` holding `value`, for a synthetic row; none for a list or opaque field,
/// which the row keeps at its default.
fn synthetic(kind: replay_data::rows::Kind, value: u8) -> Option<replay_data::rows::Field<'static>> {
    use replay_data::rows::{Field, Kind as Wire};
    Some(match kind {
        Wire::U8 => Field::U8(value),
        Wire::I8 => Field::I8(value as i8),
        Wire::U16 => Field::U16(u16::from(value)),
        Wire::I16 => Field::I16(i16::from(value)),
        Wire::U32 => Field::U32(u32::from(value)),
        Wire::I32 => Field::I32(i32::from(value)),
        Wire::U64 => Field::U64(u64::from(value)),
        Wire::I64 => Field::I64(i64::from(value)),
        Wire::Bool => Field::Bool(value % 2 == 1),
        Wire::OptionalI32 => Field::OptionalI32(Some(i32::from(value))),
        Wire::OptionalU32 => Field::OptionalU32(Some(u32::from(value))),
        Wire::OptionalI64 => Field::OptionalI64(Some(i64::from(value))),
        Wire::OptionalU64 => Field::OptionalU64(Some(u64::from(value))),
        Wire::Bytes => Field::Bytes(TEXTS[usize::from(value) % TEXTS.len()]),
        Wire::U32List | Wire::U64List | Wire::Opaque => return None,
    })
}

/// Writes the rows it is handed into `table` of `store`.
struct Put<'s> {
    store: &'s mut Store,
    table: TableId,
}

impl replay_data::rows::RowsVisit for Put<'_> {
    type Output = io::Result<()>;

    fn rows<R: replay_data::rows::RowCodec>(self, rows: &[R]) -> io::Result<()> {
        for row in rows {
            crate::testing::put(self.store, self.table, row, &TEXTS)?;
        }
        Ok(())
    }
}

/// Writes `table`'s row `key`, each scalar and text field `value(column)`, its lists and opaque
/// values their defaults; a row its type refuses so is not written.
fn synthetic_row(store: &mut Store, table: TableId, key: u128, value: impl Fn(usize) -> u8) {
    let kinds = table.schema();
    let mask = kinds.iter().enumerate().filter(|&(column, &kind)| column < 64 && synthetic(kind, 0).is_some()).fold(0_u64, |mask, (column, _)| mask | 1 << column);
    let field = |column: usize| kinds.get(column).and_then(|&kind| synthetic(kind, value(column))).unwrap_or(replay_data::rows::Field::NotRecorded);
    if let Ok(rows) = table.rows_from_wire_fields(&[key], mask, &mut |_, column| Ok(field(column))) {
        let _ = rows.visit(Put { store, table });
    }
}

/// A few keys `table` can hold.
fn synthetic_keys(table: TableId) -> Vec<u128> {
    let bits = table.key_bits();
    if bits == 0 {
        return vec![0];
    }
    [1_u128, 2, 3, 1 << 32 | 1, 2 << 32 | 2].into_iter().filter(|&key| bits >= 128 || key >> bits == 0).take(4).collect()
}

/// Every state table `writers` read and none of them writes.
fn synthetic_inputs(writers: &[Box<dyn Writer>]) -> Vec<TableId> {
    let logs = replay_data::rows::logs();
    let owned: std::collections::HashSet<usize> = writers.iter().flat_map(|writer| writer.tables().iter().map(|output| output.table().index())).collect();
    let mut inputs: Vec<TableId> = writers
        .iter()
        .flat_map(|writer| writer.reads().iter().map(|read| read.table))
        .filter(|table| !logs[table.index()] && !owned.contains(&table.index()))
        .collect();
    inputs.sort_unstable_by_key(|table| table.index());
    inputs.dedup();
    inputs
}

/// Every definition `writers` declare, held.
fn synthetic_definitions(writers: &[Box<dyn Writer>]) -> Definitions {
    let mut held = Definitions::read_none()
        .with_areas(Ok(vec![1_000; 16]))
        .with_needs(Ok(installed_content_core::unit_leader::needs::SubUnitNeeds::default()))
        .with_regions(Ok(definitions::ProvinceRegions::new([(1, &[0, 1, 2, 3, 4][..]), (2, &[4, 5, 6, 7, 8][..])])));
    for uses in writers.iter().flat_map(|writer| writer.definitions()) {
        if let Definition::Define(define) = uses.definition {
            let value = match define.form {
                definitions::Form::Int32 => 240,
                definitions::Form::Fixed5 => 25_000,
            };
            held = held.with_define(define, Ok(value));
        }
    }
    held
}

/// Hour `hour` of a synthetic campaign of `inputs`, each table's rows made of small numbers so
/// owners, slots and handles meet. One hour in seven moves and reads nothing; another moves
/// Countries and CountryLinks alone (what names and resolves); in the rest each table moves every
/// fourth hour and is read unchanged two hours later. A move shifts every cell and every third
/// one drops a row; tables are read in part for stretches, and Countries is not recorded at hours
/// 40 and 41, so the writers requiring it miss those hours.
fn synthetic_hour(store: &mut Store, inputs: &[TableId], phases: &mut [u32], hour: i32) {
    store.begin_hour(hour, if hour == 0 { &TEXTS[..] } else { &[] }).unwrap();
    let quiet = hour % 7 == 3;
    let naming = hour % 7 == 5;
    let decided = if quiet { hour - 1 } else { hour };
    for (at, &table) in inputs.iter().enumerate() {
        let index = table.index();
        let step = (hour + at as i32).rem_euclid(4);
        let moves = hour == 0 || (!quiet && if naming { matches!(table, TableId::Countries | TableId::CountryLinks) } else { step == 0 });
        let read = moves || (!quiet && !naming && step == 2);
        let status = if table == TableId::Countries && (40..42).contains(&decided) {
            Status::NotRecorded
        } else if (decided / 11 + at as i32).rem_euclid(6) == 5 {
            Status::Truncated { missing: 1 }
        } else {
            Status::Complete
        };
        let sampled = if read { Some(hour) } else { store.view(index).sampled_at() };
        store.status(index, status, read, sampled, read).unwrap();
        if !moves {
            continue;
        }
        if hour > 0 {
            phases[at] += 1;
        }
        let phase = phases[at];
        for (row, key) in synthetic_keys(table).into_iter().enumerate() {
            if row == 2 && phase % 3 == 2 {
                crate::testing::remove(store, table, key).unwrap();
                continue;
            }
            synthetic_row(store, table, key, |column| ((index * 7 + row * 3 + column * 5) as u32 + phase).rem_euclid(4) as u8);
        }
    }
    store.settle();
}

/// One table of a writer as a store holds it: its status, read hour and rows.
type HeldTable = (Status, Option<i32>, Vec<(u128, Vec<String>)>);

fn held_table(store: &Store, table: TableId) -> HeldTable {
    let view = store.view(table.index());
    (view.status(), view.sampled_at(), crate::testing::rows(store, table))
}

/// The writers write each table the schema declares derived once, and no recorded table.
#[test]
fn the_writers_write_exactly_the_derived_tables() {
    let derived: Vec<usize> = TableId::ALL.into_iter().filter(|table| table.is_derived()).map(TableId::index).collect();
    assert_eq!(crate::writers().tables(), derived);
}

/// Every writer of the journal door, each called only when it is due (a read moved as it watches
/// it, or it declares an hourly duty) and each called every hour, writes every one of its tables
/// alike at every hour, statuses and read hours included, and keeps the same resume state: one
/// that under-declares its reads, or depends on the hour without saying so, differs here by name.
/// The campaign crosses day and month boundaries, hours that move nothing, hours that move only
/// the naming inputs, inputs read unchanged, read in part and missed.
#[test]
fn every_writer_writes_the_tables_it_wrote_when_called_every_hour() {
    const HOURS: i32 = 800;
    let door = |every: bool| {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let writers: Vec<Box<dyn Writer>> =
            crate::writers().writers.into_iter().map(|inner| Box::new(Counted { inner, every, calls: calls.clone() }) as Box<dyn Writer>).collect();
        (writers, calls)
    };
    let ((every, every_calls), (gated, gated_calls)) = (door(true), door(false));
    let named: Vec<(&'static str, &'static [Output])> = gated.iter().map(|writer| (writer.name(), writer.tables())).collect();
    let inputs = synthetic_inputs(&gated);
    let held = synthetic_definitions(&gated);
    let mut runs = [every, gated].map(|writers| {
        let mut writers = Writers::new(writers);
        writers.define(held.clone());
        writers.start_run();
        (crate::testing::store(), writers, vec![0_u32; inputs.len()])
    });
    let mut differ: Vec<String> = Vec::new();
    let mut wrote: std::collections::HashSet<&'static str> = std::collections::HashSet::new();
    for hour in 0..HOURS {
        for (store, writers, phases) in &mut runs {
            synthetic_hour(store, &inputs, phases, hour);
            writers.hour(1, hour, hour == 0, store).unwrap();
            store.end_hour(&mut replay_columnar::unit::Discard).unwrap();
        }
        let [(every, ..), (gated, ..)] = &runs;
        for &(name, tables) in &named {
            let prefix = format!("{name}: ");
            if differ.iter().any(|seen| seen.starts_with(&prefix)) {
                continue;
            }
            for table in tables.iter().map(|output| output.table()) {
                let (before, after) = (held_table(every, table), held_table(gated, table));
                if !after.2.is_empty() {
                    wrote.insert(name);
                }
                if before != after {
                    differ.push(format!("{prefix}{table:?} at hour {hour}: called every hour {before:?}, called when due {after:?}"));
                    break;
                }
            }
        }
    }
    let silent: Vec<&str> = named.iter().map(|&(name, _)| name).filter(|name| !wrote.contains(name)).collect();
    eprintln!("Writers that held no row in the synthetic campaign: {silent:?}");
    assert!(differ.is_empty(), "{} writers differ:\n{}", differ.len(), differ.join("\n"));
    let [(_, every, _), (_, gated, _)] = &runs;
    assert!(every.save().unwrap() == gated.save().unwrap(), "the writers' resume state differs");
    let (every_calls, gated_calls) = (every_calls.load(std::sync::atomic::Ordering::Relaxed), gated_calls.load(std::sync::atomic::Ordering::Relaxed));
    assert!(gated_calls < every_calls, "{gated_calls} calls when due against {every_calls} every hour");
}

/// The log the toy writer appends to: a number, a text, opaque bytes and an optional number.
const TOY: TableId = TableId::NuclearStrikes;
const TOY_KINDS: [Kind; 4] = [Kind::U64, Kind::Bytes, Kind::Opaque, Kind::OptionalI32];

/// What the toy writer appended (each row's hour, ordinal, key and cells as a reader of the log
/// shows them) and forgot (each ordinal's hour), in the order it did them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Toyed {
    appended: Vec<(i32, u64, u128, Vec<String>)>,
    forgotten: Vec<(i32, u64)>,
}

impl Toyed {
    /// As a reader of the log hands it: each hour's forgets ascending.
    fn sorted(mut self) -> Self {
        self.forgotten.sort_unstable();
        self
    }

    /// The log's count and the ordinals it lists once hour `hour` is over.
    fn at(&self, hour: i32) -> (u64, Vec<u64>) {
        let appended: Vec<u64> = self.appended.iter().filter(|row| row.0 <= hour).map(|row| row.1).collect();
        let forgotten: std::collections::HashSet<u64> = self.forgotten.iter().filter(|row| row.0 <= hour).map(|row| row.1).collect();
        (appended.len() as u64, appended.into_iter().filter(|ordinal| !forgotten.contains(ordinal)).collect())
    }
}

type Toys = std::sync::Arc<std::sync::Mutex<Toyed>>;

/// The toy log keyed by the run, and by the hour its keys carry at bit 32.
const TOY_RUN: &[Output] = &[Output::Log(TOY, Keys::Run)];
const TOY_HOURLY: &[Output] = &[Output::Log(TOY, Keys::Hour(32))];

/// A writer whose one table is the toy log, declared `outputs`, appending `hour % 5` rows each
/// hour (one more every 97th hour, which it forgets within the hour), each keyed by its hour and
/// place, and forgetting by ordinal: every third row it appends in the next hour, by the ordinal
/// its append gave, and the oldest rows the log lists while more than 40 are, by the store's listed
/// set.
struct Toy {
    /// The ordinals appended last hour that it forgets this hour.
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

/// A store of [`schema`] whose toy table is a log, writing its unit when `encoded`.
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

/// What taking a run up at a block's end needs: the unit's length and pages then, and the
/// encoder's, the writers' and the strings' state.
struct Commit {
    length: usize,
    pages: usize,
    encoder: Vec<u8>,
    writers: Vec<u8>,
    next: u32,
}

/// `hours` of the run's hours, each told the store and its writers and ended into `unit`, each
/// block's end ended as the recorder ends it: the commit at the last block's end in them.
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

/// The toy log read back from `unit` by a span of hours: what it appended and forgot, and the
/// strings its rows name, by id.
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

/// The toy log's count and listed ordinals at the end of `hour`, as a seek reads them: the
/// restore at the end of the block before (none in the run's first), then the hour's block from
/// its start to it.
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

/// A writer appending to a log and forgetting from it by ordinal, over two blocks: every row it
/// appended reads back with its cells and strings, in its hour, and every forget in its hour; the
/// log's restore at each block's end and a seek to every hour list exactly the rows appended and
/// not forgotten by then, rows forgotten in the hour they came (the run's first among them) and
/// rows appended in one block and forgotten in the next included; alike whether the log holds its
/// keys for the run or the hour.
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

/// A writer appending to and forgetting from a log, taken up from the file at its first block's
/// end (the log's restore, the encoder's and the writers' state: the keys a log keyed by the run
/// was given, none of one keyed by its hour, and the toy's own), writes the second block byte for
/// byte as a run never stopped.
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

/// A log keyed by its hour, appended to over many hours and across a block's end, holds no key
/// past the hour it was given in: the writers' state at each block's end holds none of its keys,
/// where the same log keyed by the run holds a range for every hour that appended, growing with
/// the run.
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

/// How [`Misusing`] misuses its tables in its second hour.
#[derive(Clone, Copy, Debug)]
enum Misuse {
    SetLog,
    DeleteLog,
    AppendState,
    ForgetState,
    KeyAgain,
    /// Of the log keyed by its hour.
    KeyAgainInHour,
    KeyOfAnotherHour,
    ForgetNever,
    ForgetAgain,
}

/// A writer of the toy log and Campaign: in hour 0 rows 1 and 2 appended and the second forgotten,
/// and Campaign's row set; from hour 1 a row keyed by its hour appended and then its misuse.
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

/// A writer setting or deleting a row of its log, appending to or forgetting from its state table,
/// giving its log a key again (keyed by its hour, one given that same hour), giving a log keyed by
/// its hour a key of another hour, or forgetting a row the log never appended or forgot before
/// fails that hour by name and is withheld for the rest of the run: every table of it unavailable for
/// the reason, nothing of the hour landed (its row appended first included), the recording going
/// on.
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

/// A writer that declares `tables` and writes nothing.
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

/// A writer declaring a table a log that the schema's LOGS holds as state, state that LOGS holds
/// a log, a log the store holds as state, or a log's keys carrying their hour past a key's bits,
/// is refused by name at its run's first hour, before any writer runs.
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
