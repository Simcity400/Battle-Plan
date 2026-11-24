use recorder_derive_framework::{
    forces::invalid,
    framework::{Columns, DERIVED_STRINGS, Definition, Definitions, Hour, Output, Read, Rows, Uses, Value, Writer, Writers, definitions},
    testing::{self, schedule::{Memory, apply, apply_with, reason, rows_of, store}},
};
use recorder_derive_world::identity::TagHolders;
use replay_columnar::{leaf::Writer as Bytes, manifest::Status, state::{Cell, Kind}, store::Store};
use replay_data::rows::{TABLE_COUNT, TableId, column};
use std::io;

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

#[test]
fn writers_run_withhold_and_resume_by_their_own_rules() {
    let mut store = store();
    const BROKE: &str = "the recorder stopped working this out: it broke";
    let mut writers = Writers::new(vec![Box::new(TagHolders::default()), Box::new(Failing { fails: 2 })]);
    writers.start_run();
    let holders = TableId::TagHolders.index();
    let campaign = TableId::Campaign.index();
    apply(&mut store, 0, &[(9, 3, 0), (7, 3, 0), (4, 5, 1)], &[(9, 3), (7, 8), (4, 6)], Status::Complete);
    writers.hour(1, 0, true, &mut store).unwrap();
    assert_eq!(store.view(holders).status(), Status::Complete);
    assert_eq!(holder(&store, b"GER"), Some(Some((3, 9))));
    assert_eq!(holder(&store, b"ENG"), Some(None));
    assert_eq!(store.view(campaign).status(), Status::Complete);
    apply(&mut store, 1, &[(9, 3, 0), (7, 3, 0), (4, 5, 1)], &[(9, 3), (7, 8), (4, 5)], Status::Complete);
    writers.hour(1, 1, false, &mut store).unwrap();
    assert_eq!(holder(&store, b"ENG"), Some(Some((5, 4))));
    apply(&mut store, 2, &[(9, 3, 0), (4, 5, 1)], &[(9, 3), (4, 5)], Status::Complete);
    writers.hour(1, 2, false, &mut store).unwrap();
    assert_eq!(reason(&store, campaign), BROKE);
    assert_eq!(store.view(holders).status(), Status::Complete);
    apply(&mut store, 3, &[(9, 3, 0), (4, 5, 1)], &[(9, 3), (4, 5)], Status::Unavailable { reason: 0 });
    writers.hour(1, 3, false, &mut store).unwrap();
    assert_eq!(store.view(holders).status(), Status::Complete);
    assert_eq!(holder(&store, b"GER"), None);
    let table = store.view(holders);
    let unknown: Vec<u128> = table.rows().filter_map(|slot| table.key(slot)).filter(|key| (key >> 96) as u32 == u32::MAX).collect();
    assert_eq!(unknown.len(), 1, "one unknown row lives, naming Countries");
    assert_eq!((unknown[0] >> 32) as u32, 0);
    assert_eq!(reason(&store, campaign), BROKE);
    let saved = writers.save().unwrap();
    let mut resumed = Writers::new(vec![Box::new(TagHolders::default()), Box::new(Failing { fails: 99 })]);
    resumed.load(&saved, &mut store).unwrap();
    apply(&mut store, 4, &[(9, 3, 0)], &[(9, 3)], Status::Complete);
    resumed.hour(1, 4, false, &mut store).unwrap();
    assert_eq!(reason(&store, campaign), BROKE);
    assert_eq!(store.view(holders).status(), Status::Complete);
    assert_eq!(holder(&store, b"ENG"), None);
    assert_eq!(holder(&store, b"GER"), Some(Some((3, 9))));
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
    assert_eq!(reason(&store, holders), "another version of the recorder worked this out, and this version cannot take up its saved work", "never the writer's name");
}

#[test]
fn tag_holders_number_rows_alike_whatever_order_their_countries_came_in() {
    const TAGS: u32 = 64;
    let names: Vec<Vec<u8>> = (0..TAGS).map(|tag| format!("T{tag:02}").into_bytes()).collect();
    let appends: Vec<&[u8]> = names.iter().map(Vec::as_slice).collect();
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
    let epochs: Vec<u32> = written[0].0.iter().map(|(key, _)| (key >> 64) as u32).collect();
    assert_eq!(epochs.iter().filter(|&&epoch| epoch >= TAGS).count(), TAGS as usize / 2);
    assert_eq!(written[0], written[1], "the same hours number the same rows alike");
}

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

type Given = (TableId, Vec<(u128, Vec<(usize, i64)>)>);

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

type Unit = (Vec<u8>, Vec<replay_columnar::unit::BlockPage>);

fn lane_unit(every: bool) -> (Unit, usize, [usize; 3]) {
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let lane = crate::writer_list()
        .into_iter()
        .map(|inner| Box::new(Counted { inner, every, calls: calls.clone() }) as Box<dyn Writer>)
        .collect();
    let kinds = testing::kinds();
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

#[test]
fn the_lane_s_writers_write_the_bytes_they_wrote_when_called_every_hour() {
    let (gated, gated_calls, held) = lane_unit(false);
    let (every, every_calls, _) = lane_unit(true);
    assert!(held.iter().all(|&rows| rows > 0), "TagHolders, ArmyTotals and TerritoryTotals hold rows: {held:?}");
    assert!(gated_calls < every_calls, "{gated_calls} calls gated against {every_calls} every hour");
    assert!(gated.0 == every.0, "the unit's bytes differ");
    assert_eq!(gated.1, every.1);
}

const TEXTS: [&[u8]; 3] = [b"GER", b"ENG", b"FRA"];

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

struct Put<'s> {
    store: &'s mut Store,
    table: TableId,
}

impl replay_data::rows::RowsVisit for Put<'_> {
    type Output = io::Result<()>;

    fn rows<R: replay_data::rows::RowCodec>(self, rows: &[R]) -> io::Result<()> {
        for row in rows {
            testing::put(self.store, self.table, row, &TEXTS)?;
        }
        Ok(())
    }
}

fn synthetic_row(store: &mut Store, table: TableId, key: u128, value: impl Fn(usize) -> u8) {
    let kinds = table.schema();
    let mask = kinds.iter().enumerate().filter(|&(column, &kind)| column < 64 && synthetic(kind, 0).is_some()).fold(0_u64, |mask, (column, _)| mask | 1 << column);
    let field = |column: usize| kinds.get(column).and_then(|&kind| synthetic(kind, value(column))).unwrap_or(replay_data::rows::Field::NotRecorded);
    if let Ok(rows) = table.rows_from_wire_fields(&[key], mask, &mut |_, column| Ok(field(column))) {
        let _ = rows.visit(Put { store, table });
    }
}

fn synthetic_keys(table: TableId) -> Vec<u128> {
    let bits = table.key_bits();
    if bits == 0 {
        return vec![0];
    }
    [1_u128, 2, 3, 1 << 32 | 1, 2 << 32 | 2].into_iter().filter(|&key| bits >= 128 || key >> bits == 0).take(4).collect()
}

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

fn synthetic_definitions(writers: &[Box<dyn Writer>]) -> Definitions {
    let mut held = Definitions::read_none()
        .with_areas(Ok(vec![1_000; 16]))
        .with_needs(Ok(content_reader_key_names::unit_leader::needs::SubUnitNeeds::default()))
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
                testing::remove(store, table, key).unwrap();
                continue;
            }
            synthetic_row(store, table, key, |column| ((index * 7 + row * 3 + column * 5) as u32 + phase).rem_euclid(4) as u8);
        }
    }
    store.settle();
}

type HeldTable = (Status, Option<i32>, Vec<(u128, Vec<String>)>);

fn held_table(store: &Store, table: TableId) -> HeldTable {
    let view = store.view(table.index());
    (view.status(), view.sampled_at(), testing::rows(store, table))
}

#[test]
fn every_writer_writes_the_tables_it_wrote_when_called_every_hour() {
    const HOURS: i32 = 800;
    let door = |every: bool| {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let writers: Vec<Box<dyn Writer>> =
            crate::writer_list().into_iter().map(|inner| Box::new(Counted { inner, every, calls: calls.clone() }) as Box<dyn Writer>).collect();
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
        (testing::store(), writers, vec![0_u32; inputs.len()])
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
