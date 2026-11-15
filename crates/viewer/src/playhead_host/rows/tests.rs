use super::{Cadence, Recording, RowReader, Rows};
use crate::playhead_host::{open, units::Units};
use playhead::{
    Settings, Timeline,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::{Store, Value},
    unit::{BLOCK, BlockPage, Encoder, PartSink, Settings as Writing, reader::Extents},
};
use replay_data::{
    Hour,
    rows::{Field, List, TableId, TableRows, tables},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, SharedView, TableView};
use std::{io, sync::Arc, time::Duration};

const COUNTRIES: TableId = TableId::Countries;
pub(crate) const COMBATS: TableId = TableId::LandCombats;
const STRINGS: [&[u8]; 3] = [b"GER", b"ENG", b"the collector could not read it"];
pub(crate) const RECORDING: Recording = Recording { branch: 7 };
const LIMIT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Memory {
    bytes: Vec<u8>,
    pages: Vec<BlockPage>,
}

impl PartSink for Memory {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64> {
        let offset = self.bytes.len() as u64;
        self.bytes.extend_from_slice(stored);
        Ok(offset)
    }

    fn block(&mut self, page: BlockPage) -> io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn read_back(&mut self, offset: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.extend_from_slice(&self.bytes[offset as usize..offset as usize + stored as usize]);
        Ok(())
    }
}

struct Recorded {
    bytes: Vec<u8>,
    runs: Vec<Run>,
    kinds: Vec<Vec<Kind>>,
}

impl Pages for Recorded {
    fn extents(&self, _run: usize) -> &dyn Extents {
        &self.bytes
    }

    fn runs(&self) -> &[Run] {
        &self.runs
    }

    fn kinds(&self, table: usize) -> Option<&[Kind]> {
        self.kinds.get(table).map(Vec::as_slice)
    }
}

/// A cell as written, and as today's frame reader hands it to the row codec.
#[derive(Clone)]
pub(crate) enum Put {
    Cell(Cell<'static>),
    Unrecorded,
}

impl Put {
    fn field(&self) -> Field<'static> {
        match self {
            Self::Unrecorded => Field::NotRecorded,
            Self::Cell(cell) => match *cell {
                Cell::U8(value) => Field::U8(value),
                Cell::I8(value) => Field::I8(value),
                Cell::U16(value) => Field::U16(value),
                Cell::I16(value) => Field::I16(value),
                Cell::U32(value) => Field::U32(value),
                Cell::I32(value) => Field::I32(value),
                Cell::U64(value) => Field::U64(value),
                Cell::I64(value) => Field::I64(value),
                Cell::Bool(value) => Field::Bool(value),
                Cell::OptionalI32(value) => Field::OptionalI32(value),
                Cell::OptionalU32(value) => Field::OptionalU32(value),
                Cell::OptionalI64(value) => Field::OptionalI64(value),
                Cell::OptionalU64(value) => Field::OptionalU64(value),
                Cell::Bytes(id) => Field::Bytes(STRINGS[id as usize]),
                Cell::U32List(items) => Field::U32List(List::Native(items)),
                Cell::U64List(items) => Field::U64List(List::Native(items)),
                Cell::Opaque(_) => Field::NotRecorded,
            },
        }
    }
}

/// A country row: slot, key, and its ten cells; `stability` moves.
fn country(key: u128, tag: u32, stability: i64, missing: bool) -> (u128, Vec<Put>) {
    let power = if missing { Cell::OptionalI64(None) } else { Cell::OptionalI64(Some(-25_000)) };
    let cells = vec![
        Put::Cell(Cell::U32(key as u32 * 2)),
        Put::Cell(Cell::Bytes(tag)),
        Put::Cell(power),
        Put::Cell(Cell::I64(1_000)),
        Put::Cell(Cell::I64(stability)),
        Put::Cell(Cell::I64(-3_000)),
        Put::Cell(Cell::OptionalI64(Some(12_000))),
        Put::Cell(Cell::U8(1)),
        Put::Cell(Cell::I32(-77)),
        if missing { Put::Unrecorded } else { Put::Cell(Cell::OptionalI32(Some(3))) },
    ];
    (key, cells)
}

pub(crate) fn combat(key: u128, day: i32) -> (u128, Vec<Put>) {
    static ATTACKERS: [u64; 3] = [u64::MAX, 4, 1 << 40];
    static CLASSES: [u32; 2] = [9, 0];
    let cells = vec![
        Put::Cell(Cell::U32(4_100)),
        Put::Cell(Cell::I32(12)),
        Put::Cell(Cell::I32(day)),
        Put::Cell(Cell::U64List(&ATTACKERS)),
        Put::Cell(Cell::U64List(&[])),
        Put::Cell(Cell::U32List(&CLASSES)),
        Put::Cell(Cell::U32List(&[])),
    ];
    (key, cells)
}

/// What one hour records of the two tables.
struct Hourly {
    countries: Status,
    country_rows: Vec<(u128, Vec<Put>)>,
    combats: Vec<(u128, Vec<Put>)>,
}

/// The rows by hour: Countries complete with a missing optional and an unrecorded cell (hours 0
/// to 3), one row's stability moving at hour 2, truncated at 4, unavailable at 5, not recorded at
/// 6; LandCombats' day moving every hour.
fn hourly(hour: u32) -> Hourly {
    let countries = match hour {
        4 => Status::Truncated { missing: 4 },
        5 => Status::Unavailable { reason: 2 },
        6 => Status::NotRecorded,
        _ => Status::Complete,
    };
    let stability = if hour >= 2 { 50_000 } else { 40_000 };
    Hourly {
        countries,
        country_rows: vec![country(9, 1, stability, false), country(2, 0, 10_000, true), country(5, 1, 0, false)],
        combats: vec![combat(30, hour as i32), combat(4, -(hour as i32))],
    }
}

pub(crate) fn write(store: &mut Store, table: TableId, rows: &[(u128, Vec<Put>)]) {
    let mut changes = store.table(table.index()).unwrap();
    for (slot, (key, cells)) in rows.iter().enumerate() {
        let slot = slot as u32;
        if changes.slot(*key).is_none() {
            changes.insert_at(slot, *key).unwrap();
        }
        for (column, put) in cells.iter().enumerate() {
            match put {
                Put::Cell(cell) => changes.write(slot, column, *cell).unwrap(),
                Put::Unrecorded => changes.put_value(slot, column, Value::Unrecorded).unwrap(),
            }
        }
    }
}

/// `hours` hours from game hour 0 of every schema table, Countries and LandCombats written.
pub(crate) fn record(name: &str, hours: u32) -> Units {
    record_with(name, hours, |store, at| {
        let hour = hourly(at);
        store.status(COUNTRIES.index(), hour.countries, true, Some(at as i32), true).unwrap();
        if at == 0 || at == 2 {
            write(store, COUNTRIES, &hour.country_rows);
        }
        store.status(COMBATS.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        write(store, COMBATS, &hour.combats);
        for (table, rows) in [(TableId::CountryLinks, links()), (TableId::HumanControllers, humans())] {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
            if at == 0 {
                write(store, table, &rows);
            }
        }
    })
}

/// `hours` hours from game hour 0 of every schema table, each hour's writes made by `hour`.
pub(crate) fn record_with(name: &str, hours: u32, mut hour: impl FnMut(&mut Store, u32)) -> Units {
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing::at(3)).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..hours {
        store.begin_hour(at as i32, if at == 0 { &STRINGS[..] } else { &[] }).unwrap();
        hour(&mut store, at);
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    if !hours.is_multiple_of(BLOCK) {
        store.end_run(&mut memory).unwrap();
    }
    let run = Run::new(0, memory.pages).unwrap();
    let kinds = Arc::new(kinds);
    let pages: Arc<dyn Pages> = Arc::new(Recorded { bytes: memory.bytes, runs: vec![run], kinds: (*kinds).clone() });
    let timeline = Timeline::whole(&*pages).unwrap();
    Units { name: format!("rows test {name}"), pages, timeline, kinds }
}

/// An hour a step.
pub(crate) fn hourly_steps() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0 }
}

/// The rows the table's codec gives for `rows`, ascending by key, as today's frame reader hands
/// it each cell.
fn codec(table: TableId, rows: &[(u128, Vec<Put>)]) -> TableRows {
    let mut sorted: Vec<&(u128, Vec<Put>)> = rows.iter().collect();
    sorted.sort_by_key(|(key, _)| *key);
    let keys: Vec<u128> = sorted.iter().map(|(key, _)| *key).collect();
    table.rows_from_wire(&keys, &mut |row, column| Ok(sorted[row].1[column].field())).unwrap()
}

fn at(reader: &RowReader, hour: Hour) -> Arc<DecodedFrame> {
    reader.wait(|frame| frame.hour == hour, LIMIT).unwrap()
}

#[test]
fn rows_from_a_mirror_equal_the_rows_the_tables_codec_gives_with_their_status() {
    let units = record("codec", 8);
    let entry = open(units, hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let reader = rows.reader(&[COUNTRIES, COMBATS], Cadence::EveryStep);
    rows.settled();
    for hour in 0..7 {
        rows.drive(hour);
        entry.playhead().seek(hour);
        let frame = at(&reader, hour);
        let wanted = hourly(hour as u32);
        assert_eq!(frame.branch, RECORDING.branch);
        assert_eq!(*frame.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &wanted.combats)), "hour {hour}");
        let countries = codec(COUNTRIES, &wanted.country_rows);
        let expected = match wanted.countries {
            Status::Complete => TableView::Complete(countries),
            Status::Truncated { missing } => TableView::Truncated { rows: countries, missing },
            Status::Unavailable { .. } => TableView::Unavailable { reason: "the collector could not read it".into() },
            Status::NotRecorded => TableView::NotRecorded { mode: CaptureMode::Full },
        };
        assert_eq!(*frame.tables[COUNTRIES.index()], expected, "hour {hour}");
        let sampled = frame.tables[COMBATS.index()].sampled_at();
        assert_eq!(sampled, Some(hour), "hour {hour}");
    }
    let rows = frame_rows(&at(&reader, 6));
    assert!(rows.is_empty(), "a table not recorded holds no rows");
}

fn frame_rows(frame: &DecodedFrame) -> Vec<u128> {
    match &*frame.tables[COUNTRIES.index()] {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.rows::<tables::Countries>().unwrap().iter().map(|row| u128::from(row.id)).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn a_table_a_step_leaves_alone_keeps_its_rows_and_a_touched_one_gets_new_ones() {
    let units = record("arcs", 4);
    let entry = open(units, hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let reader = rows.reader(&[COUNTRIES, COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(0);
    entry.playhead().seek(0);
    let mut frames = vec![at(&reader, 0)];
    for hour in 1..4 {
        rows.drive(hour);
        frames.push(at(&reader, hour));
    }
    let same = |table: TableId, from: usize| SharedView::ptr_eq(&frames[from].tables[table.index()], &frames[from + 1].tables[table.index()]);
    assert!(same(COUNTRIES, 0), "Countries is unchanged from hour 0 to 1");
    assert!(!same(COUNTRIES, 1), "a stability moved at hour 2");
    assert!(same(COUNTRIES, 2), "Countries is unchanged from hour 2 to 3");
    for from in 0..3 {
        assert!(!same(COMBATS, from), "a combat's day moves every hour");
    }
    let timings = rows.timings();
    assert!(timings.is_none());
}

/// LandCombats' rows by hour: six from hour 0, one row's day moving at hour 1, and at hour 2 one
/// row gone and another come in its slot. Countries is written at hour 0 alone.
fn one_row_at_a_time(hour: u32) -> Vec<(u128, Vec<Put>)> {
    let keys: &[u128] = if hour < 2 { &[1, 2, 3, 4, 5, 6] } else { &[1, 3, 4, 5, 6, 9] };
    keys.iter().map(|&key| combat(key, i32::from(key == 4 && hour >= 1))).collect()
}

fn record_one_row_at_a_time(name: &str) -> Units {
    record_with(name, 3, |store, at| {
        for table in [COUNTRIES, COMBATS] {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
        match at {
            0 => {
                write(store, COUNTRIES, &hourly(0).country_rows);
                write(store, COMBATS, &one_row_at_a_time(0));
            }
            1 => {
                let mut changes = store.table(COMBATS.index()).unwrap();
                let slot = changes.slot(4).unwrap();
                changes.write(slot, 2, Cell::I32(1)).unwrap();
            }
            _ => {
                let mut changes = store.table(COMBATS.index()).unwrap();
                let gone = changes.slot(2).unwrap();
                changes.delete(gone).unwrap();
                let slot = changes.insert(9).unwrap();
                assert_eq!(slot, gone, "the new row takes the slot of the one gone");
                for (column, put) in combat(9, 0).1.iter().enumerate() {
                    let Put::Cell(cell) = put else { unreachable!("every combat cell is recorded") };
                    changes.write(slot, column, *cell).unwrap();
                }
            }
        }
    })
}

#[test]
fn a_step_that_touches_one_row_makes_that_row_again_and_no_other_table() {
    let entry = open(record_one_row_at_a_time("patch"), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let reader = rows.reader(&[COUNTRIES, COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(0);
    entry.playhead().seek(0);
    let mut frames = vec![at(&reader, 0)];
    for hour in 1..3 {
        rows.drive(hour);
        frames.push(at(&reader, hour));
    }
    for (hour, frame) in frames.iter().enumerate() {
        assert_eq!(*frame.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &one_row_at_a_time(hour as u32))), "hour {hour}");
        assert_eq!(*frame.tables[COUNTRIES.index()], TableView::Complete(codec(COUNTRIES, &hourly(0).country_rows)), "hour {hour}");
    }
    let timings = rows.timings().unwrap();
    let made = |table: TableId| -> Vec<usize> { timings.builds.get(&table.index()).into_iter().flatten().map(|&(_, rows)| rows).collect() };
    assert_eq!(made(COMBATS), [6, 1, 1], "whole at the seek, then the row whose day moved, then the row that came");
    assert_eq!(made(COUNTRIES), [3], "no step after the seek touched Countries");
}

/// A telling reader takes with each frame the rows made anew since the frame it took before: every
/// table made whole at the seek; with the frame of hour 1 not taken, at hour 2 the row whose day
/// moved at 1 with the row gone and the row come at 2, keys with the columns written (every column
/// for a row gone or come); taken again, nothing. A reader that is not telling is told nothing.
#[test]
fn a_telling_reader_takes_the_rows_made_anew_since_the_frame_it_took_before() {
    use std::collections::BTreeMap;
    let entry = open(record_one_row_at_a_time("told"), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let telling = rows.telling(&[COUNTRIES, COMBATS], Cadence::EveryStep);
    let plain = rows.reader(&[COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(0);
    entry.playhead().seek(0);
    at(&telling, 0);
    let (frame, told) = telling.take().expect("a frame at hour 0");
    assert_eq!(frame.hour, 0);
    assert_eq!(told, BTreeMap::from([(COUNTRIES, None), (COMBATS, None)]), "made whole at the seek");
    rows.drive(1);
    at(&telling, 1);
    rows.drive(2);
    at(&telling, 2);
    let (frame, told) = telling.take().expect("a frame at hour 2");
    assert_eq!(frame.hour, 2);
    let every = super::view::EVERY;
    assert_eq!(told, BTreeMap::from([(COMBATS, Some(BTreeMap::from([(2, every), (4, 1 << 2), (9, every)])))]), "row 4's day at hour 1, row 2 gone and row 9 come at hour 2");
    assert_eq!(telling.take().map(|(_, told)| told), Some(BTreeMap::new()), "taken again, nothing more");
    at(&plain, 2);
    assert_eq!(plain.take().map(|(_, told)| told), Some(BTreeMap::new()), "a reader that is not telling is told nothing");
}

#[test]
fn two_readers_of_one_table_share_its_rows_made_once_for_both() {
    let entry = open(record("shared", 4), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let combats = rows.reader(&[COMBATS], Cadence::EveryStep);
    let both = rows.reader(&[COUNTRIES, COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(0);
    entry.playhead().seek(0);
    for hour in 0..4 {
        rows.drive(hour);
        let (one, other) = (at(&combats, hour), at(&both, hour));
        assert!(SharedView::ptr_eq(&one.tables[COMBATS.index()], &other.tables[COMBATS.index()]), "hour {hour}");
    }
    let timings = rows.timings().unwrap();
    assert_eq!(timings.builds.get(&COMBATS.index()).map(Vec::len), Some(4), "LandCombats moves every hour and is made once an hour");
}

#[test]
fn readers_of_one_cadence_opened_apart_are_due_at_the_same_ticks() {
    let at = Duration::from_millis;
    let cadence = Cadence::PerSecond(10);
    assert_eq!(cadence.tick(at(130)), cadence.tick(at(170)), "shown at 130 and 170 ms, both are next due at 200");
    assert!(cadence.tick(at(200)) > cadence.tick(at(170)));
    assert_eq!(cadence.tick(at(1_000)).zip(cadence.tick(at(0))).map(|(late, early)| late - early), Some(10), "ten ticks a second");
    assert_eq!(Cadence::EveryStep.tick(at(5)), None);
}

/// A reader at a panel's slow cadence gets the head's rows at once after a seek and when the head
/// stops, never at its next cadence tick: a panel shows the head within a frame or two there.
/// With its cadence held, those frames can only be the ones due at once.
#[test]
fn a_slow_reader_gets_the_heads_rows_at_once_after_a_seek_and_at_a_pause() {
    let units = record("at once", 12);
    let entry = open(units, hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let reader = rows.reader(&[COUNTRIES, COMBATS], Cadence::PerSecond(1));
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    at(&reader, 1);
    rows.hold_cadence();
    rows.drive(6);
    entry.playhead().seek(6);
    reader.wait(|frame| frame.hour == 6, LIMIT).expect("a seek hands the sought hour at once");
    rows.drive(7);
    reader.wait(|frame| frame.hour == 7, LIMIT).expect("a head that stops hands its hour at once");
}

#[test]
fn a_table_the_reader_did_not_declare_is_a_loud_failure() {
    let units = record("undeclared", 2);
    let entry = open(units, hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let reader = rows.reader(&[COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    let frame = at(&reader, 1);
    assert!(frame.tables[COUNTRIES.index()].is_not_declared());
    assert!(frame.complete::<tables::LandCombats>().is_ok());
    let read = std::panic::catch_unwind(|| frame.complete::<tables::Countries>().map(<[_]>::len));
    if cfg!(debug_assertions) {
        assert!(read.is_err(), "a debug build panics");
    } else {
        let error = read.unwrap().unwrap_err().to_string();
        assert!(error.contains(replay_frame::frame::NOT_DECLARED), "{error}");
    }
}

#[test]
fn a_reader_that_closes_retires_its_tables_and_rows_are_never_made_on_the_callers_thread() {
    let units = record("retire", 3);
    let entry = open(units, hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let both = rows.reader(&[COUNTRIES, COMBATS], Cadence::PerSecond(4));
    let combats = rows.reader(&[COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    at(&both, 1);
    at(&combats, 1);
    drop(both);
    rows.drive(2);
    at(&combats, 2);
    let held = rows.held();
    assert_eq!(held.mirrors.iter().map(|(table, _)| *table).collect::<Vec<_>>(), vec![COMBATS], "Countries is let go");
    let timings = rows.timings().unwrap();
    assert_eq!(timings.built_on.as_deref(), Some("rows at the head"));
    assert_ne!(std::thread::current().name(), Some("rows at the head"));
    assert!(timings.builds.contains_key(&COUNTRIES.index()));
}

/// Each country's raw id and canonical slot.
fn links() -> Vec<(u128, Vec<Put>)> {
    [(9, 18), (2, 4), (5, 10)].into_iter().map(|(id, slot)| (id, vec![Put::Cell(Cell::U32(slot))])).collect()
}

/// Humans control country 9.
fn humans() -> Vec<(u128, Vec<Put>)> {
    vec![(9, vec![Put::Cell(Cell::I8(1))]), (5, vec![Put::Cell(Cell::I8(0))])]
}

/// A frame of no table, as today's frame reader holds an hour it has not decoded.
fn nothing(hour: Hour) -> Arc<DecodedFrame> {
    Arc::new(DecodedFrame { branch: RECORDING.branch, hour, tables: std::array::from_fn(|_| replay_frame::frame::not_selected()), metrics: Default::default() })
}

#[test]
fn the_counter_owner_filter_reads_rows_at_the_head_while_the_playhead_is_on() {
    use dashboard::protocol::CounterFilter;
    let entry = open(record("owners", 3), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let mut owners = crate::map_views::CounterOwners::default();
    let today = nothing(1);
    assert_eq!(owners.get(Some(&*today), None, &CounterFilter::Players), None, "today's frame holds no table");
    assert_eq!(owners.get(None, Some((&entry, RECORDING)), &CounterFilter::Players), None, "nothing resolved while the rows load");
    let watch = rows.reader(&[COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    at(&watch, 1);
    let players = owners.get(None, Some((&entry, RECORDING)), &CounterFilter::Players);
    assert_eq!(players.as_deref(), Some(&[18].into_iter().collect()), "country 9 is human and sits at slot 18");
    let chosen = owners.get(None, Some((&entry, RECORDING)), &CounterFilter::Countries { tags: vec!["ENG".into()] });
    assert_eq!(chosen.as_deref(), Some(&[18, 10].into_iter().collect()));
    assert_eq!(owners.get(Some(&*today), None, &CounterFilter::Players), None, "off, it reads today's frame again");
}

#[test]
fn a_reader_at_the_head_learns_the_tables_it_reads_and_closes_when_a_frame_does_not_ask() {
    let entry = open(record("tooltip", 40), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let watch = rows.reader(&[COMBATS], Cadence::EveryStep);
    rows.settled();
    // Every step: a frame the table's reset lands on at an hour it is not complete (4 to 6) is
    // followed by the next hour's, never by a wait for a cadence longer than the loop.
    let mut hover = super::AtHead::with(&[], Cadence::EveryStep);
    let today = nothing(1);
    let tags = |frame: &Arc<DecodedFrame>| frame.complete::<tables::Countries>().map(|rows| rows.iter().map(|row| String::from_utf8_lossy(&row.tag).into_owned()).collect::<Vec<_>>()).ok();
    rows.drive(1);
    entry.playhead().seek(1);
    at(&watch, 1);
    // Rows at the head once they are held and the read found every table declared; today's frame until then.
    let ask = |hover: &mut super::AtHead| hover.frame(&entry, RECORDING).and_then(|rows| hover.discover(|| tags(&rows))).unwrap_or_else(|| tags(&today));
    let mut read = Vec::new();
    for hour in 2..38 {
        read.push(ask(&mut hover));
        hover.end_frame();
        if read.last().is_some_and(Option::is_some) {
            break;
        }
        rows.drive(hour);
        at(&watch, hour);
    }
    assert_eq!(read[0], None, "today's frame while the reader opens");
    assert_eq!(read.last().cloned().flatten(), Some(vec!["GER".to_owned(), "ENG".to_owned(), "ENG".to_owned()]), "Countries by key: 2, 5, 9, once the table it learned joins");
    assert!(hover.is_open());
    hover.end_frame();
    assert!(!hover.is_open(), "a frame that did not ask closes it");
}

/// One playhead plays every branch of its recording: rows asked for another branch come from a
/// worker of their own, stamped with it, and a reader at the head asked for it opens on it.
#[test]
fn rows_at_the_head_carry_the_branch_they_are_asked_for() {
    let entry = open(record("branches", 12), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let other = Recording { branch: RECORDING.branch + 1 };
    let (first, second) = (Rows::of(&entry, RECORDING).unwrap(), Rows::of(&entry, other).unwrap());
    assert!(!Arc::ptr_eq(&first, &second), "rows asked for branch {} are the worker stamped with branch {}", other.branch, RECORDING.branch);
    let watches = [first.reader(&[COMBATS], Cadence::EveryStep), second.reader(&[COMBATS], Cadence::EveryStep)];
    first.settled();
    second.settled();
    let mut hour = 1;
    let drive = |hour: Hour| {
        first.drive(hour);
        second.drive(hour);
        for watch in &watches {
            at(watch, hour);
        }
    };
    first.drive(hour);
    second.drive(hour);
    entry.playhead().seek(hour);
    for watch in &watches {
        at(watch, hour);
    }
    let mut window = super::AtHead::new(&[COMBATS], Cadence::EveryStep);
    for recording in [RECORDING, other] {
        let frame = loop {
            if let Some(frame) = window.frame(&entry, recording) {
                break frame;
            }
            assert!(hour < 11, "no rows at the head for branch {} by hour {hour}", recording.branch);
            hour += 1;
            drive(hour);
        };
        assert_eq!(frame.branch, recording.branch, "the window's reader asked for branch {} at hour {}", recording.branch, frame.hour);
    }
}

mod combat;
mod hover;

#[test]
fn a_reader_of_some_columns_or_of_keys_alone_holds_only_those_and_any_other_field_is_refused_by_name() {
    use super::Need;
    use replay_data::rows::{CountryRow, mask};
    let units = record("columns", 4);
    let entry = open(units, hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let stability = rows.reader_of(&[Need::columns(COUNTRIES, mask::<CountryRow>(&["stability_raw"])), Need::keys(COMBATS)], Cadence::EveryStep);
    rows.settled();
    rows.drive(3);
    entry.playhead().seek(3);
    let frame = at(&stability, 3);
    let named = mask::<CountryRow>(&["stability_raw"]);
    let countries = frame.complete_fields::<tables::Countries>(named).unwrap();
    let wanted = hourly(3);
    let mut sorted = wanted.country_rows.clone();
    sorted.sort_by_key(|(key, _)| *key);
    assert_eq!(countries.iter().map(|row| u128::from(row.id)).collect::<Vec<_>>(), sorted.iter().map(|(key, _)| *key).collect::<Vec<_>>(), "every live row by key");
    for (row, (_, cells)) in countries.iter().zip(&sorted) {
        assert!(matches!(cells[4], Put::Cell(Cell::I64(value)) if value == row.stability_raw), "the named column is the recording's");
    }
    // A read of every field, or of a field not named, is refused, and discovery names what it read.
    let (whole, missed) = replay_frame::frame::discovering_fields(|| frame.complete::<tables::Countries>().map(<[_]>::len));
    assert!(whole.is_err(), "never served: {whole:?}");
    assert_eq!(missed, vec![(COUNTRIES, u64::MAX)]);
    let tag = mask::<CountryRow>(&["tag"]);
    let (tagged, missed) = replay_frame::frame::discovering_fields(|| frame.complete_fields::<tables::Countries>(tag | named).map(<[_]>::len));
    assert!(tagged.is_err());
    assert_eq!(missed, vec![(COUNTRIES, tag | named)], "the fields asked for, to widen the need");
    if cfg!(debug_assertions) {
        let refused = std::panic::catch_unwind(|| frame.complete_fields::<tables::Countries>(tag).map(<[_]>::len)).unwrap_err();
        let message = refused.downcast_ref::<String>().cloned().unwrap_or_default();
        assert!(message.contains("Countries") && message.contains("tag"), "refused by the field's name: {message}");
    }
    let combats = frame.complete_fields::<tables::LandCombats>(0).unwrap();
    assert_eq!(combats.len(), 2, "keys alone: both live combats");
    let held = rows.held();
    let bytes = |table: TableId| held.mirrors.iter().find(|(held, _)| *held == table).map(|(_, bytes)| *bytes).unwrap();
    let narrow = (bytes(COUNTRIES), bytes(COMBATS));
    let entry = open(record("columns whole", 4), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let whole = rows.reader(&[COUNTRIES, COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(3);
    entry.playhead().seek(3);
    at(&whole, 3);
    let held = rows.held();
    let bytes = |table: TableId| held.mirrors.iter().find(|(held, _)| *held == table).map(|(_, bytes)| *bytes).unwrap();
    assert!(narrow.0 < bytes(COUNTRIES) && narrow.1 < bytes(COMBATS), "one column and keys alone hold less than every column: {narrow:?} against {:?}", (bytes(COUNTRIES), bytes(COMBATS)));
}

/// A map drawing the stepped harness's frames, its clock playing at the harness's step as the
/// harness sets it: the rows follow the head the map takes, hour by hour, not the clock.
#[test]
fn rows_follow_the_head_a_map_takes_not_the_clock() {
    let units = record("map head", 8);
    let entry = open(units, crate::playhead_host::settings()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let mut side = crate::playhead_host::MapSide::new(Arc::clone(&entry), Vec::new(), crate::layers::View::default()).unwrap();
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let reader = rows.reader(&[COUNTRIES, COMBATS], Cadence::EveryStep);
    rows.settled();
    side.anchor(0, false, 0.0, true);
    side.anchor(0, true, 24.0 * crate::playhead_host::settings().rate, false);
    for hour in 0..7 {
        side.catch_up(hour).unwrap_or_else(|error| panic!("hour {hour}: {error}"));
        let frame = at(&reader, hour);
        assert_eq!(frame.tables[COMBATS.index()].sampled_at(), Some(hour), "hour {hour}");
    }
}

/// Two playheads in one process (the dashboard maps run several), each with its own rows worker
/// and reader, stepped at once from two threads: neither waits on the other.
#[test]
fn two_playheads_stepping_at_once_each_reach_every_hour() {
    let step = |name: &'static str| {
        move || {
            let entry = open(record(name, 40), hourly_steps()).unwrap();
            entry.set_capture(CaptureMode::Full);
            let rows = Rows::of(&entry, RECORDING).unwrap();
            let reader = rows.reader(&[COUNTRIES, COMBATS], Cadence::EveryStep);
            rows.settled();
            rows.drive(1);
            entry.playhead().seek(1);
            for hour in 1..36 {
                rows.drive(hour);
                reader.wait(|frame| frame.hour == hour, Duration::from_secs(20)).map_err(|error| format!("{name} at hour {hour}: {error}"))?;
            }
            Ok::<_, String>(())
        }
    };
    let first = std::thread::spawn(step("two playheads, the first"));
    let second = std::thread::spawn(step("two playheads, the second"));
    let (first, second) = (first.join().unwrap(), second.join().unwrap());
    assert_eq!((first, second), (Ok(()), Ok(())));
}

/// A head that jumps past the newest step built (a late frame, a driven head) is reached: the
/// step built to catch up ends at the head, not past it, so a reader at a held head is never left
/// a step behind.
#[test]
fn a_head_that_jumps_past_the_newest_step_is_reached_while_it_holds() {
    let entry = open(record("jump", 40), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let reader = rows.reader(&[COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    at(&reader, 1);
    rows.drive(20);
    let reached = reader.wait(|frame| frame.hour == 20, Duration::from_secs(10));
    assert!(reached.is_ok(), "{reached:?}");
}

/// A reader that declares again while the worker holds a frame made for its old declaration
/// (paused between making and publishing) is never given that frame: it has none until one made
/// for its new tables is published, and that is the one publish after the declaration.
#[test]
fn a_frame_made_for_a_declaration_the_reader_since_replaced_is_never_published() {
    let entry = open(record("redeclared mid publish", 8), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let reader = rows.reader(&[COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    at(&reader, 1);
    let pause = rows.pause_before_publish();
    rows.drive(2);
    assert!(pause.holding(std::time::Instant::now() + LIMIT), "the worker makes the reader's frame of hour 2 and holds it unpublished");
    let seen = reader.newest().expect("the frame of hour 1").published;
    reader.declare(&[COUNTRIES, COMBATS]);
    assert!(reader.frame().is_none(), "declared again, its frame is cleared");
    pause.release();
    let holds = |frame: &DecodedFrame| !frame.tables[COUNTRIES.index()].is_not_declared();
    assert!(reader.frame().is_none_or(|frame| holds(&frame)), "after the release, no frame without the table it declared");
    let newer = reader.wait_newer(seen, LIMIT).unwrap_or_else(|| panic!("a frame made for its new declaration is published within {LIMIT:?}: {:?}", rows.error()));
    assert!(holds(&newer.frame), "the first frame published after the declaration holds Countries, not the one made for LandCombats alone (hour {})", newer.frame.hour);
    assert_eq!(newer.published, seen + 1, "the frame made for the old declaration was dropped, not published");
    assert_eq!(*newer.frame.tables[COUNTRIES.index()], TableView::Complete(codec(COUNTRIES, &hourly(newer.frame.hour as u32).country_rows)), "Countries as recorded at the frame's hour");
}

/// The head moves to hour 3 and stays: a wait begun after the frame of hour 3 was published, and a
/// reader declared after the head's last move, both get hour 3's rows. The worker looks at the
/// head again on its own, so neither depends on a wake sent while it waited.
#[test]
fn a_reader_that_starts_waiting_after_the_heads_last_move_gets_the_heads_rows() {
    let entry = open(record("late wait", 6), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let first = rows.reader(&[COUNTRIES, COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(3);
    entry.playhead().seek(3);
    at(&first, 3);
    let newest = first.newest().expect("hour 3 published");
    let late = first.wait_newer(newest.published - 1, LIMIT).expect("a frame newer than the one before hour 3's");
    assert_eq!((late.frame.hour, late.published), (3, newest.published), "a wait begun after the publish returns it at once");
    let declared_late = rows.reader(&[COUNTRIES], Cadence::PerSecond(4));
    assert_eq!(at(&declared_late, 3).hour, 3, "a reader declared after the head's last move gets the head's rows");
}

/// A head driven before a seek is not the rows' head after it. On the base the worker advanced the
/// playhead to it after the seek, the step after the reset ran from the sought hour through that
/// old head, and a drive to an hour before it was never reached (the orders test that failed once
/// in two runs: rows at 4, driven to 8).
#[test]
fn a_head_driven_before_a_seek_does_not_carry_the_rows_past_the_next_drive() {
    let entry = open(record("stale drive", 40), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let reader = rows.reader(&[COMBATS], Cadence::PerSecond(4));
    rows.settled();
    rows.drive(30);
    entry.playhead().seek(30);
    at(&reader, 30);
    entry.playhead().seek(4);
    reader.wait(|frame| frame.hour >= 4 && frame.hour < 8, LIMIT).expect("the sought hour's rows, not the old drive's");
    rows.drive(8);
    at(&reader, 8);
}

/// LandCombats' rows by hour: 24 combats whose three numbers move every hour, so each step
/// rewrites most of the table.
fn churning(hour: u32) -> Vec<(u128, Vec<Put>)> {
    (1..=24_u32)
        .map(|key| {
            let cells = vec![
                Put::Cell(Cell::U32(hour * 100 + key)),
                Put::Cell(Cell::I32(hour as i32)),
                Put::Cell(Cell::I32(-(hour as i32) - key as i32)),
                Put::Cell(Cell::U64List(&[])),
                Put::Cell(Cell::U64List(&[])),
                Put::Cell(Cell::U32List(&[])),
                Put::Cell(Cell::U32List(&[])),
            ];
            (u128::from(key), cells)
        })
        .collect()
}

/// The hour the head runs to while the worker is held.
const FAR: Hour = 160;

/// `hours` hours of [`churning`] LandCombats, and Countries written at hour 0.
fn record_churning(name: &str, hours: u32) -> Units {
    record_with(name, hours, |store, at| {
        store.status(COMBATS.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        write(store, COMBATS, &churning(at));
        store.status(COUNTRIES.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        if at == 0 {
            write(store, COUNTRIES, &hourly(0).country_rows);
        }
    })
}

/// A worker reading LandCombats through `cadences`, held before it takes a step at hour 1 while the
/// map (a worker of another branch whose reader takes every step) drives the head hour by hour to
/// [`FAR`], then let go with its head at [`FAR`]. Each reader's frame of [`FAR`], the steps the worker
/// took and those it applied since it was held, and its landings.
fn left_behind(name: &str, cadences: &[Cadence]) -> (Vec<Arc<DecodedFrame>>, u64, usize, u64) {
    let entry = open(record_churning(name, FAR as u32 + 4), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let readers: Vec<RowReader> = cadences.iter().map(|&cadence| rows.reader(&[COMBATS], cadence)).collect();
    let map = Rows::of(&entry, Recording { branch: RECORDING.branch + 1 }).unwrap();
    let watch = map.reader(&[COUNTRIES], Cadence::EveryStep);
    rows.settled();
    map.settled();
    rows.drive(1);
    map.drive(1);
    entry.playhead().seek(1);
    for reader in &readers {
        at(reader, 1);
    }
    at(&watch, 1);
    let hold = rows.hold_before_take();
    assert!(hold.holding(std::time::Instant::now() + LIMIT), "the worker holds before it takes a step: {:?}", rows.error());
    let before = rows.timings().unwrap();
    for hour in 2..=FAR {
        map.drive(hour);
        at(&watch, hour);
    }
    rows.drive(FAR);
    hold.release();
    let frames = readers.iter().map(|reader| at(reader, FAR)).collect();
    let after = rows.timings().unwrap();
    (frames, after.steps - before.steps, after.apply.len() - before.apply.len(), after.landed - before.landed)
}

/// A worker left 159 steps behind while the head ran on lands at the head when it is let go:
/// its reader at a cadence gets the head's rows from its tables joined whole there, and the steps
/// it missed are passed over, not applied one by one (on the base it applied all 159).
#[test]
fn a_worker_far_behind_the_head_lands_there_without_applying_each_step_it_missed() {
    let (frames, taken, applied, landed) = left_behind("far behind", &[Cadence::PerSecond(4)]);
    assert_eq!(*frames[0].tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &churning(FAR as u32))), "the head's rows");
    assert!(taken >= FAR as u64 - 1, "the steps it was behind by were taken: {taken}");
    assert!(applied * 2 < FAR as usize - 1, "{applied} steps applied of the {} it missed", FAR - 1);
    assert_eq!(landed, 1, "it landed once");
}

/// A reader of every step is never served by a landing: the worker applies each step it missed, in
/// order, and gives every reader the head's rows.
#[test]
fn a_worker_far_behind_with_a_reader_of_every_step_applies_each_step_it_missed() {
    let (frames, taken, applied, landed) = left_behind("far behind, every step", &[Cadence::EveryStep, Cadence::PerSecond(4)]);
    for frame in &frames {
        assert_eq!(*frame.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &churning(FAR as u32))), "the head's rows");
    }
    assert_eq!(landed, 0, "no landing while a reader takes every step");
    assert_eq!(applied as u64, taken, "every step taken is applied");
    assert!(taken >= FAR as u64 - 1, "{taken} steps taken");
}

/// A worker left behind again after it landed lands again, every time landing costs less than
/// applying what it missed: three times it is held while the map drives the head [`FAR`] hours on.
/// The first wake after lands; the second takes the tables' arrival and the steps after it, and no
/// worker lands while a table waits for its reset; the third lands again, at what its first landing
/// cost against what its applies cost a byte. On the base the first landing was the only one: a
/// worker that had not stood at the head since applied every step it missed, and at 1,440 hours a
/// second fell 1,121 hours behind for good.
#[test]
fn a_worker_left_behind_again_after_it_landed_lands_again() {
    let entry = open(record_churning("lands again", 3 * FAR as u32 + 4), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let reader = rows.reader(&[COMBATS], Cadence::PerSecond(4));
    let map = Rows::of(&entry, Recording { branch: RECORDING.branch + 1 }).unwrap();
    let watch = map.reader(&[COUNTRIES], Cadence::EveryStep);
    rows.settled();
    map.settled();
    rows.drive(1);
    map.drive(1);
    entry.playhead().seek(1);
    at(&reader, 1);
    at(&watch, 1);
    let until = || std::time::Instant::now() + LIMIT;
    let mut held = rows.hold_before_take();
    assert!(held.holding(until()), "held at hour 1: {:?}", rows.error());
    let mut skips = Vec::new();
    for to in [FAR, 2 * FAR, 3 * FAR] {
        for hour in to - FAR + 1..=to {
            map.drive(hour);
            at(&watch, hour);
        }
        rows.drive(to);
        if to < 3 * FAR {
            let next = rows.hold_after_taking(1);
            held.release();
            assert!(next.holding(until()), "held after its wake to hour {to}: {:?}", rows.error());
            held = next;
        } else {
            held.release();
        }
        skips.push(rows.timings().map(|timings| timings.landed));
    }
    let frame = at(&reader, 3 * FAR);
    assert_eq!(*frame.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &churning(3 * FAR as u32))), "the head's rows");
    let landed = rows.timings().unwrap().landed;
    assert_eq!((skips[0], skips[1], landed), (Some(1), Some(1), 2), "a landing at the first wake, none while its tables arrive, another at the third: {skips:?}");
}

/// A table read by every one of its columns and then declared whole by another reader names no new
/// column: its rows stay, and both readers get the next hour's. On the base the worker dropped its
/// mirror for the wider mask while it declared the same columns to the playhead, which sent no
/// reset: every reader of the table waited for ever (the far view's overlay, on ScriptVariables).
#[test]
fn a_table_read_by_every_column_then_declared_whole_keeps_its_rows_for_every_reader() {
    use super::Need;
    let entry = open(record("whole after every column", 6), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let columns = entry.kinds()[COMBATS.index()].len();
    let named = rows.reader_of(&[Need::columns(COMBATS, (1 << columns) - 1)], Cadence::EveryStep);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    at(&named, 1);
    let whole = rows.reader(&[COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(2);
    let wanted = TableView::Complete(codec(COMBATS, &hourly(2).combats));
    assert_eq!(*at(&whole, 2).tables[COMBATS.index()], wanted, "the reader that declared it whole");
    assert_eq!(*at(&named, 2).tables[COMBATS.index()], wanted, "the reader that named every column");
}

/// A table the recording holds nothing of arrives at once as not recorded, by name; it withholds
/// no frame from its reader, and the reader's other tables come with it.
#[test]
fn a_table_the_recording_holds_nothing_of_comes_as_not_recorded_and_withholds_no_frame() {
    let entry = open(record("never held", 4), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let reader = rows.reader(&[COMBATS, TableId::Divisions], Cadence::PerSecond(4));
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    let frame = at(&reader, 1);
    assert_eq!(*frame.tables[TableId::Divisions.index()], TableView::NotRecorded { mode: CaptureMode::Full }, "refused by name, not waited for");
    assert_eq!(*frame.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &hourly(1).combats)));
    assert!(reader.arriving().is_empty(), "nothing it declared is still awaited: {:?}", reader.arriving());
}

/// [`record_with`] written with `writing`.
fn record_written(name: &str, hours: u32, writing: Writing, mut hour: impl FnMut(&mut Store, u32)) -> Units {
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, writing).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..hours {
        store.begin_hour(at as i32, if at == 0 { &STRINGS[..] } else { &[] }).unwrap();
        hour(&mut store, at);
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    if !hours.is_multiple_of(BLOCK) {
        store.end_run(&mut memory).unwrap();
    }
    let run = Run::new(0, memory.pages).unwrap();
    let kinds = Arc::new(kinds);
    let pages: Arc<dyn Pages> = Arc::new(Recorded { bytes: memory.bytes, runs: vec![run], kinds: (*kinds).clone() });
    let timeline = Timeline::whole(&*pages).unwrap();
    Units { name: format!("rows test {name}"), pages, timeline, kinds }
}

/// LandCombats: eight combats at hour 0, six of them gone at hour 1, and nothing after, so its
/// first block's end packs its two rows into the first slots: written to restore every table whole
/// at every block's end (the run's first hour restores each, and a table is restored again only
/// after `restore_blocks` blocks, and packed only by a whole restore). Countries: a stability that
/// moves every hour.
fn record_packed(name: &str, hours: u32) -> Units {
    record_written(name, hours, Writing { pack_margin: 0, restore_blocks: 1, whole_every: 1, ..Writing::at(3) }, |store, at| {
        for table in [COUNTRIES, COMBATS] {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
        write(store, COUNTRIES, &[country(9, 1, i64::from(at), false)]);
        match at {
            0 => write(store, COMBATS, &(1..=8).map(|key| combat(key, 3)).collect::<Vec<_>>()),
            1 => {
                let mut changes = store.table(COMBATS.index()).unwrap();
                for key in 1..=6 {
                    let slot = changes.slot(key).unwrap();
                    changes.delete(slot).unwrap();
                }
            }
            _ => {}
        }
    })
}

/// A table no step touches keeps its rows, the same allocation in every frame, and is made once,
/// though a block's end renumbers its slots and every step touches another table. On the base a
/// renumber counted as a change: the table was made again, whole.
#[test]
fn a_table_no_step_touches_is_made_once_and_keeps_its_allocation_across_a_renumber() {
    let last = BLOCK as Hour + 10;
    let units = record_packed("renumbered", last as u32 + 2);
    let page = &units.pages.runs()[0].pages[0];
    assert!(page.tables[COMBATS.index()].packed, "the recording packs LandCombats at its first block's end");
    assert!(!page.tables[COUNTRIES.index()].packed, "and not Countries");
    let entry = open(units, hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let reader = rows.reader(&[COUNTRIES, COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(2);
    entry.playhead().seek(2);
    let first = at(&reader, 2);
    let mut hour = 2;
    while hour < last {
        hour = (hour + 40).min(last);
        rows.drive(hour);
        let frame = at(&reader, hour);
        assert!(SharedView::ptr_eq(&frame.tables[COMBATS.index()], &first.tables[COMBATS.index()]), "LandCombats is the allocation of hour 2 at hour {hour}");
        assert!(!SharedView::ptr_eq(&frame.tables[COUNTRIES.index()], &first.tables[COUNTRIES.index()]), "Countries moved by hour {hour}");
    }
    let frame = at(&reader, last);
    assert_eq!(*frame.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &[combat(7, 3), combat(8, 3)])));
    let timings = rows.timings().unwrap();
    assert!(timings.renumbered.get(&COMBATS.index()).is_some_and(|&count| count >= 1), "the block's end renumbered LandCombats: {:?}", timings.renumbered);
    assert_eq!(timings.makes.get(&COMBATS.index()).map(Vec::len), Some(1), "made once, at the seek: {:?}", timings.makes.get(&COMBATS.index()));
    assert!(timings.makes.get(&COUNTRIES.index()).map_or(0, Vec::len) > 2, "Countries is made as it moves");
}

/// LandCombats with `count` combats at hour 0, the day of the combats `moved(hour)` names set to
/// the hour from hour 1 on.
fn record_moving(name: &str, hours: u32, count: u128, moved: fn(u32) -> Vec<u128>) -> Units {
    record_with(name, hours, move |store, at| {
        store.status(COMBATS.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        if at == 0 {
            write(store, COMBATS, &(1..=count).map(|key| combat(key, 0)).collect::<Vec<_>>());
            return;
        }
        let mut changes = store.table(COMBATS.index()).unwrap();
        for key in moved(at) {
            let slot = changes.slot(key).unwrap();
            changes.write(slot, 2, Cell::I32(at as i32)).unwrap();
        }
    })
}

/// The combats' rows at `hour` of [`record_moving`].
fn moving(hour: u32, count: u128, moved: fn(u32) -> Vec<u128>) -> Vec<(u128, Vec<Put>)> {
    (1..=count).map(|key| combat(key, (1..=hour).rev().find(|&at| moved(at).contains(&key)).map_or(0, |at| at as i32))).collect()
}

/// Twenty of 24 combats move at hour 1, one at hour 2.
fn most_then_one(hour: u32) -> Vec<u128> {
    if hour == 1 { (1..=20).collect() } else { vec![1] }
}

/// A table with most of its rows touched is patched for those rows, not made whole: on the base a
/// view with half its rows or more touched was made whole (24, 24, 1 rows made).
#[test]
fn a_table_with_most_of_its_rows_touched_is_patched_while_patching_pays() {
    let entry = open(record_moving("most rows", 4, 24, most_then_one), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let reader = rows.reader(&[COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(0);
    entry.playhead().seek(0);
    at(&reader, 0);
    for hour in 1..3 {
        rows.drive(hour);
        let frame = at(&reader, hour);
        assert_eq!(*frame.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &moving(hour as u32, 24, most_then_one))), "hour {hour}");
    }
    let timings = rows.timings().unwrap();
    let made: Vec<usize> = timings.builds[&COMBATS.index()].iter().map(|&(_, rows)| rows).collect();
    assert_eq!(made, [24, 20, 1], "whole at the seek, then the 20 rows moved, then the one");
}

/// The combat whose key is the hour moves.
fn one_an_hour(hour: u32) -> Vec<u128> {
    vec![u128::from(hour)]
}

/// Where a view's LandCombats rows are.
fn combat_rows(view: &SharedView) -> usize {
    match view.held() {
        TableView::Complete(rows) => rows.rows::<tables::LandCombats>().unwrap().as_ptr() as usize,
        other => panic!("LandCombats holds no rows: {other:?}"),
    }
}

/// Once no frame holds a view the worker made earlier, its next make patches that view's rows in
/// place for every row made anew since it, instead of copying the newest view a frame holds: the
/// rows of hour 0 are the buffer of hour 2's, and those of hour 1 of hour 3's. On the base every
/// patch copied the newest view's rows.
#[test]
fn a_view_no_frame_holds_any_more_is_patched_in_place_and_not_copied() {
    let entry = open(record_moving("in place", 5, 24, one_an_hour), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let reader = rows.reader(&[COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(0);
    entry.playhead().seek(0);
    let mut buffers = vec![combat_rows(&at(&reader, 0).tables[COMBATS.index()])];
    for hour in 1..4 {
        rows.drive(hour);
        let frame = at(&reader, hour);
        assert_eq!(*frame.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &moving(hour as u32, 24, one_an_hour))), "hour {hour}");
        buffers.push(combat_rows(&frame.tables[COMBATS.index()]));
    }
    let timings = rows.timings().unwrap();
    use super::{Make, Whole};
    assert_eq!(timings.makes[&COMBATS.index()], [Make::Whole(Whole::First), Make::Copied, Make::InPlace, Make::InPlace]);
    assert_eq!((buffers[2], buffers[3]), (buffers[0], buffers[1]), "hour 2 is made in the rows of hour 0, hour 3 in those of hour 1");
    let made: Vec<usize> = timings.builds[&COMBATS.index()].iter().map(|&(_, rows)| rows).collect();
    assert_eq!(made, [24, 1, 2, 2], "each patch in place makes the rows moved since the view it patches");
    assert!(rows.held().spares.iter().any(|&(table, bytes)| table == COMBATS && bytes > 0), "the view of hour 2, held by no frame, is kept and counted: {:?}", rows.held().spares);
}

/// What each way costs decides: until a table's makes are measured a patched row costs what a row
/// made does and a copied row an eighth of it; once they are measured, the rows patched and copied
/// are weighed by what they cost against every row made.
#[test]
fn patching_pays_by_what_each_way_costs_the_table_not_by_a_fixed_half() {
    use super::Costs;
    let whole = 1_000.0;
    let unmeasured = Costs::default();
    assert!(unmeasured.patching(800, 1_000) < whole, "800 rows patched and 1,000 copied at an eighth");
    assert!(unmeasured.patching(900, 1_000) >= whole, "900 rows patched and 1,000 copied at an eighth");
    assert!(unmeasured.patching(999, 0) < whole, "999 rows patched in place");
    let measured = Costs { whole: Some(100.0), patch: Some(120.0), copy: Some(30.0) };
    assert!(measured.patching(800, 0) < whole, "800 rows at 120 against 1,000 at 100");
    assert!(measured.patching(900, 0) >= whole, "900 rows at 120 against 1,000 at 100");
    assert!(measured.patching(500, 1_000) < whole, "500 at 120 and 1,000 copied at 30 against 1,000 at 100");
    assert!(measured.patching(600, 1_000) >= whole, "600 at 120 and 1,000 copied at 30 against 1,000 at 100");
    assert_eq!(measured.bound(1_000), 1_667, "noting stops past twice the rows a patch could beat");
}

/// The combats alive at `hour` of [`record_turning`], each with its day: 24 at hour 0; at every
/// fifth hour the combat keyed a fifth of it is gone and one keyed 100 more comes; each hour the
/// combat keyed the hour's place in 24 moves.
fn turning(hour: u32) -> Vec<(u128, Vec<Put>)> {
    let moved = |key: u32| (1..=hour).rev().find(|&at| at % 24 + 1 == key && at < 5 * key).map_or(0, |at| at as i32);
    let kept = (1..=24).filter(|&key| 5 * key > hour).map(|key| combat(u128::from(key), moved(key)));
    let come = (1..=hour / 5).map(|fifth| combat(u128::from(100 + fifth), (5 * fifth) as i32));
    kept.chain(come).collect()
}

fn record_turning(name: &str, hours: u32) -> Units {
    record_with(name, hours, |store, at| {
        store.status(COMBATS.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        if at == 0 {
            write(store, COMBATS, &turning(0));
            return;
        }
        let mut changes = store.table(COMBATS.index()).unwrap();
        if at % 5 == 0 {
            if let Some(slot) = changes.slot(u128::from(at / 5)) {
                changes.delete(slot).unwrap();
            }
            let key = u128::from(100 + at / 5);
            let slot = changes.insert(key).unwrap();
            for (column, put) in combat(key, at as i32).1.iter().enumerate() {
                let Put::Cell(cell) = put else { unreachable!("every combat cell is recorded") };
                changes.write(slot, column, *cell).unwrap();
            }
        }
        if let Some(slot) = changes.slot(u128::from(at % 24 + 1)) {
            changes.write(slot, 2, Cell::I32(at as i32)).unwrap();
        }
    })
}

/// A reader of every step gets every step's rows in order, cell for cell as the table's codec
/// makes them, while rows come, go and move, the patches are made in place or on a copy as the
/// frames held allow, and a reader at a cadence reads the same table.
#[test]
fn a_reader_of_every_step_gets_each_steps_rows_in_order_however_they_were_made() {
    let last: Hour = 60;
    let entry = open(record_turning("turning", last as u32 + 2), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let every = rows.reader(&[COMBATS], Cadence::EveryStep);
    let paced = rows.reader(&[COMBATS], Cadence::PerSecond(4));
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    let mut before = at(&every, 1).hour;
    let mut kept = Vec::new();
    for hour in 2..=last {
        rows.drive(hour);
        let frame = at(&every, hour);
        assert_eq!(*frame.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &turning(hour as u32))), "hour {hour}");
        assert!(frame.hour > before, "in order");
        before = frame.hour;
        if hour % 3 == 0 {
            kept.push(frame);
        }
    }
    assert_eq!(*at(&paced, last).tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &turning(last as u32))));
    let timings = rows.timings().unwrap();
    let makes = &timings.makes[&COMBATS.index()];
    assert!(makes.contains(&super::Make::InPlace) && makes.contains(&super::Make::Copied), "both ways were taken: {makes:?}");
}

/// Countries written once, at hour 0, and sampled at hour 0 every hour; LandCombats' day moving
/// every hour. A status left unset carries its run on, and a run sampled "at its own hour" is
/// sampled again every hour: a status change in every step.
fn record_still_countries(name: &str, hours: u32) -> Units {
    record_with(name, hours, |store, at| {
        store.status(COUNTRIES.index(), Status::Complete, true, Some(0), true).unwrap();
        if at == 0 {
            write(store, COUNTRIES, &hourly(0).country_rows);
        }
        store.status(COMBATS.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        write(store, COMBATS, &hourly(at).combats);
    })
}

/// A step's change set that changes nothing in a table (no row, cell, status, string or slot) is
/// not applied to its mirror: Countries, written at hour 0 alone, takes no change set after the
/// seek's reset, while LandCombats takes every step's. On the base every step's change set
/// of every table held was applied.
#[test]
fn a_change_set_that_changes_nothing_is_not_applied() {
    let last: Hour = 12;
    let entry = open(record_still_countries("still", last as u32 + 2), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let reader = rows.reader(&[COUNTRIES, COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    let first = at(&reader, 1);
    let before = rows.timings().unwrap();
    for hour in 2..=last {
        rows.drive(hour);
        let frame = at(&reader, hour);
        assert!(SharedView::ptr_eq(&frame.tables[COUNTRIES.index()], &first.tables[COUNTRIES.index()]), "hour {hour}");
        assert_eq!(*frame.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &hourly(hour as u32).combats)), "hour {hour}");
    }
    let after = rows.timings().unwrap();
    let applied = |table: TableId| after.applies.get(&table.index()).copied().unwrap_or(0) - before.applies.get(&table.index()).copied().unwrap_or(0);
    assert_eq!(applied(COUNTRIES), 0, "no change set of Countries after its reset");
    assert_eq!(applied(COMBATS), (last - 1) as u64, "each step's of LandCombats");
}

/// A reader of a slower cadence is due only on the ticks of the fastest open one, and as often a
/// second as its own cadence asks: at four a second beside ten, at 300, 500, 800 and 1,000 ms. On
/// the base its ticks fell between the faster reader's, and a table both read was made for each.
#[test]
fn a_reader_of_a_slower_cadence_is_due_on_the_fastest_readers_ticks_as_often_as_it_asks() {
    let (fast, slow) = (Cadence::PerSecond(10), Cadence::PerSecond(4));
    let ms = Duration::from_millis;
    let clock = |at: u64| super::on_fastest(ms(at), [fast, slow, Cadence::EveryStep]);
    let mut moved = Vec::new();
    for at in 1..=2_000 {
        if slow.tick(clock(at)) != slow.tick(clock(at - 1)) {
            moved.push(at);
            assert_ne!(fast.tick(clock(at)), fast.tick(clock(at - 1)), "at {at} ms the faster reader is due too");
        }
    }
    assert_eq!(moved, [300, 500, 800, 1_000, 1_300, 1_500, 1_800, 2_000], "four a second, none lost");
    assert_eq!(super::on_fastest(ms(1_234), [Cadence::EveryStep]), ms(1_234), "no cadence: the clock itself");
}

/// Two readers of different cadences share a table no step touches: the rows a reader at ten a
/// second took are the allocation a reader at four a second takes, made once.
#[test]
fn readers_of_different_cadences_share_the_rows_of_a_table_no_step_touches() {
    let last: Hour = 8;
    let entry = open(record_still_countries("cadences", last as u32 + 2), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let fast = rows.reader(&[COUNTRIES, COMBATS], Cadence::PerSecond(10));
    let slow = rows.reader(&[COUNTRIES], Cadence::PerSecond(4));
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    let first = at(&fast, 1);
    for hour in 2..=last {
        rows.drive(hour);
        at(&fast, hour);
    }
    let (fast, slow) = (at(&fast, last), slow.wait(|frame| frame.hour == last, LIMIT).unwrap());
    for frame in [&fast, &slow] {
        assert!(SharedView::ptr_eq(&frame.tables[COUNTRIES.index()], &first.tables[COUNTRIES.index()]), "the rows of hour 1 at hour {}", frame.hour);
    }
    assert_eq!(rows.timings().unwrap().makes.get(&COUNTRIES.index()).map(Vec::len), Some(1), "made once for both");
}

/// A reader's frame stands at each hour the worker applies while no step changed a table of it:
/// Countries, written at hour 0 alone, stands at every hour driven whether or not a frame of it
/// was published since; LandCombats, moved every hour, stands no further than its own frame's
/// hour; a frame a newer one replaced stands at none. On the base no reader could say so, and a
/// product of rows that stood still read as behind by every hour since its frame.
#[test]
fn a_frame_whose_tables_no_step_touched_since_stands_at_the_newest_hour_applied() {
    let last: Hour = 10;
    let entry = open(record_still_countries("stands", last as u32 + 2), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let watch = rows.reader(&[COMBATS], Cadence::EveryStep);
    let still = rows.reader(&[COUNTRIES], Cadence::PerSecond(1));
    let moving = rows.reader(&[COMBATS], Cadence::PerSecond(1));
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    let mut before = at(&watch, 1);
    for hour in 2..=last {
        rows.drive(hour);
        let now = at(&watch, hour);
        assert_eq!(watch.through(Arc::as_ptr(&before)), None, "hour {hour}: a frame replaced stands at none");
        assert_eq!(watch.through(Arc::as_ptr(&now)), Some(hour));
        let frame = still.frame().expect("Countries' rows");
        assert_eq!(still.through(Arc::as_ptr(&frame)), Some(hour), "hour {hour}: Countries' rows of hour {} stand at the hour applied", frame.hour);
        let frame = moving.frame().expect("LandCombats' rows");
        assert_eq!(moving.through(Arc::as_ptr(&frame)), Some(frame.hour), "hour {hour}: LandCombats moved since its frame");
        before = now;
    }
}

/// A wait for the rows at an hour is answered by a frame made before it that stands there: a
/// reader that waits until ready, its frame of Countries taken at hour 1 and nothing said since,
/// is made no new frame while Countries stands, and the wait for hour 6 gets hour 1's frame. On the
/// base the whole settles of the meshes, the models, the orders and the intel icons waited for a
/// frame whose own hour reached theirs: no such frame came while the rows stood (the clock
/// playing), and each waited out its ten seconds before going on with the frame it had.
#[test]
fn a_wait_for_the_rows_at_an_hour_is_answered_by_a_frame_that_stands_there() {
    let last: Hour = 6;
    let entry = open(record_still_countries("stands-wait", last as u32 + 2), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let watch = rows.reader(&[COMBATS], Cadence::EveryStep);
    let still = rows.reader(&[COUNTRIES], Cadence::Ready);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    let first = at(&still, 1);
    for hour in 2..=last {
        rows.drive(hour);
        at(&watch, hour);
    }
    let stood = still.wait_through(last, LIMIT).unwrap_or_else(|error| panic!("Countries' rows of hour 1 stand at hour {last}: {error}"));
    assert!(Arc::ptr_eq(&stood, &first), "no new frame was made: the rows stood, and the reader said nothing; got hour {}", stood.hour);
    assert_eq!((stood.hour, still.through(Arc::as_ptr(&stood))), (1, Some(last)), "a frame of hour 1, standing at hour {last}");
}

/// A reader whose product reads the hour declares the Clock: its frame never stands past its own
/// hour, whatever the Clock's rows do (this recording holds none of them), so its rows are made at
/// each hour it says it is ready for, and a wait for a later hour is answered by a frame of that
/// hour. On `ef7ddf1bb1` hour 1's frame stood at hour 6, and the intel icons' whole settle at a
/// step went on with the icons of the hour before.
#[test]
fn a_reader_of_the_clock_never_stands_past_its_frames_hour() {
    let last: Hour = 6;
    let entry = open(record_still_countries("clock-stands", last as u32 + 2), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let watch = rows.reader(&[COMBATS], Cadence::EveryStep);
    let hourly = rows.reader(&[COUNTRIES, TableId::Clock], Cadence::Ready);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    let first = at(&hourly, 1);
    for hour in 2..=last {
        rows.drive(hour);
        at(&watch, hour);
    }
    assert_eq!(hourly.through(Arc::as_ptr(&first)), Some(1), "Countries stood still, but the reader reads the hour");
    hourly.ready();
    let frame = hourly.wait_through(last, LIMIT).unwrap_or_else(|error| panic!("a frame of hour {last} once it said it is ready: {error}"));
    assert_eq!(frame.hour, last, "the hour's own frame, not hour 1's standing");
}

/// A whole settle waiting for the rows at an hour gets them though the worker publishes a frame of
/// an earlier hour while it waits: that frame (hour 2, made before the head reached 3, held at a
/// named gate until the wait for hour 3 is under way) takes no readiness from the reader, so the
/// worker makes it hour 3's. On `cff29785cb` the publish cleared the readiness the waiter had said,
/// no frame of hour 3 was made, and the wait ran out (the battle plan's whole settle in
/// `a_plan_is_kept_while_its_rows_are_unchanged_...`).
#[test]
fn a_wait_for_the_rows_at_an_hour_is_answered_though_an_earlier_frame_is_published_while_it_waits() {
    let entry = open(record_still_countries("ready-wait", 10), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let watch = rows.reader(&[COUNTRIES], Cadence::EveryStep);
    let reader = rows.reader(&[COMBATS], Cadence::Ready);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    at(&reader, 1);
    reader.ready();
    let gate = rows.pause_before_publish();
    rows.drive(2);
    assert!(gate.holding(std::time::Instant::now() + LIMIT), "the worker holds hour 2's frame before it publishes it");
    rows.drive(3);
    std::thread::scope(|scope| {
        let waiting = scope.spawn(|| reader.wait_through(3, LIMIT));
        gate.release();
        let frame = waiting.join().expect("the waiter").unwrap_or_else(|error| panic!("the rows at hour 3: {error}"));
        assert_eq!(frame.hour, 3);
    });
    drop(watch);
}

/// Frames held long and many at once (a maker's build holding its frame while steps go on) leave
/// an earlier view to patch in place however many newer ones they hold: with hour 1's frame held
/// throughout and the last three frames held at each step, every make from hour 6 on patches in
/// place the view of four hours before, making the four rows moved since, and copies nothing;
/// hour 1's frame still holds hour 1's rows, cell for cell, after twelve more makes. On the base,
/// which kept two earlier views, both were always held and every make copied the table.
#[test]
fn a_view_is_patched_in_place_on_the_newest_one_released_however_many_newer_ones_frames_hold() {
    const COUNT: u128 = 31;
    let last: Hour = 14;
    let entry = open(record_moving("held long", last as u32 + 2, COUNT, one_an_hour), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let reader = rows.reader(&[COMBATS], Cadence::EveryStep);
    rows.settled();
    rows.drive(0);
    entry.playhead().seek(0);
    at(&reader, 0);
    let mut window = std::collections::VecDeque::new();
    let mut held = None;
    for hour in 1..=last {
        rows.drive(hour);
        let frame = at(&reader, hour);
        assert_eq!(*frame.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &moving(hour as u32, COUNT, one_an_hour))), "hour {hour}");
        if hour == 1 {
            held = Some(Arc::clone(&frame));
        }
        window.push_back(frame);
        if window.len() > 3 {
            window.pop_front();
        }
    }
    let held = held.unwrap();
    assert_eq!(*held.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &moving(1, COUNT, one_an_hour))), "the frame held since hour 1 is unchanged");
    let timings = rows.timings().unwrap();
    let makes = &timings.makes[&COMBATS.index()];
    let made: Vec<usize> = timings.builds[&COMBATS.index()].iter().map(|&(_, rows)| rows).collect();
    let from = makes.len() - (last as usize - 5);
    assert!(makes[from..].iter().all(|&how| how == super::Make::InPlace), "in place from hour 6 on: {makes:?}");
    assert!(made[from..].iter().all(|&rows| rows == 4), "the four rows moved since the view patched: {made:?}");
}

/// A reader that feeds a picture and says it is ready at every step has a frame of every step's
/// hour that changed a table it reads, made in the wake that applied the step, beside the frame of
/// a reader of every step (counted over twelve steps); a table no step changes stands at the hour
/// applied without a frame. On the base the picture readers were due four or ten times a second:
/// between their ticks a step's rows were not theirs.
#[test]
fn a_reader_ready_at_every_step_has_the_rows_of_each_step_that_changes_a_table_it_reads() {
    let last: Hour = 13;
    let entry = open(record_still_countries("at the step", last as u32 + 2), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let watch = rows.reader(&[COMBATS], Cadence::EveryStep);
    let picture = rows.reader(&[COMBATS], Cadence::Ready);
    let still = rows.reader(&[COUNTRIES], Cadence::Ready);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    at(&watch, 1);
    let mut at_step = 0;
    for hour in 2..=last {
        picture.ready();
        rows.drive(hour);
        at(&watch, hour);
        at_step += i32::from(picture.frame().is_some_and(|frame| frame.hour == hour));
        let frame = still.frame().expect("Countries' rows");
        assert_eq!(still.through(Arc::as_ptr(&frame)), Some(hour), "hour {hour}");
    }
    assert_eq!(at_step, last - 1, "the picture's reader had the rows of every step's hour");
}

/// One combat moves an hour; a reader of no table steps the test hour by hour.
fn ready_world(name: &str, hours: u32) -> (Arc<crate::playhead_host::Entry>, Arc<Rows>, RowReader) {
    let entry = open(record_moving(name, hours, 64, one_an_hour), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let watch = rows.reader(&[COUNTRIES], Cadence::EveryStep);
    (entry, rows, watch)
}

/// The rows of LandCombats a telling reader was told were made anew: keys and columns.
fn told_combats(told: &super::Told) -> Option<std::collections::BTreeMap<u128, u64>> {
    told.get(&COMBATS).cloned().flatten()
}

/// A reader that is not ready is made no rows however many steps change a table it reads; once it
/// says it is, the worker's next wake makes it one frame, at the newest step applied, that holds
/// every row touched since its last: one make of the table, of the nine rows moved. On the base a
/// reader at a cadence was made rows at every tick whether it was ready or not.
#[test]
fn a_reader_not_ready_is_made_nothing_then_one_frame_of_every_row_touched_since() {
    let last: Hour = 11;
    let (entry, rows, watch) = ready_world("not ready", last as u32 + 2);
    let slow = rows.telling(&[COMBATS], Cadence::Ready);
    rows.settled();
    rows.drive(2);
    entry.playhead().seek(2);
    at(&watch, 2);
    let first = slow.wait_newer(0, LIMIT).expect("its first frame, at the seek");
    assert_eq!(first.frame.hour, 2);
    let _ = slow.take();
    let makes = || rows.timings().unwrap().builds.get(&COMBATS.index()).map_or(0, Vec::len);
    let made_at_seek = makes();
    for hour in 3..=last {
        rows.drive(hour);
        at(&watch, hour);
        assert_eq!(slow.newest().map(|newest| newest.published), Some(first.published), "hour {hour}: not ready, no frame");
    }
    assert_eq!(makes(), made_at_seek, "no rows made for a reader not ready");
    slow.ready();
    let next = slow.wait_newer(first.published, LIMIT).expect("one frame once ready");
    assert_eq!((next.frame.hour, next.published), (last, first.published + 1));
    let (_, told) = slow.take().expect("its frame");
    assert_eq!(told_combats(&told), Some((3..=last as u128).map(|key| (key, 1 << 2)).collect()), "every row moved since its last frame");
    let timings = rows.timings().unwrap();
    let builds = &timings.builds[&COMBATS.index()];
    assert_eq!(builds.len(), made_at_seek + 1, "one make");
    assert_eq!(builds.last().map(|&(_, rows)| rows), Some((last - 2) as usize), "of the rows moved since");
}

/// Two telling readers of one table, one ready at every step and one every third: each frame of
/// each holds exactly the rows touched since that reader's own last frame.
#[test]
fn readers_ready_at_their_own_pace_each_get_the_rows_touched_since_their_own_last_frame() {
    let last: Hour = 14;
    let (entry, rows, watch) = ready_world("fast and slow", last as u32 + 2);
    let fast = rows.telling(&[COMBATS], Cadence::Ready);
    let slow = rows.telling(&[COMBATS], Cadence::Ready);
    rows.settled();
    rows.drive(2);
    entry.playhead().seek(2);
    at(&watch, 2);
    let mut seen = [fast.wait_newer(0, LIMIT).unwrap().published, slow.wait_newer(0, LIMIT).unwrap().published];
    let _ = (fast.take(), slow.take());
    let mut since = 2;
    for hour in 3..=last {
        fast.ready();
        rows.drive(hour);
        at(&watch, hour);
        let newest = fast.wait_newer(seen[0], LIMIT).expect("the fast reader's frame");
        seen[0] = newest.published;
        assert_eq!(newest.frame.hour, hour);
        let (_, told) = fast.take().unwrap();
        assert_eq!(told_combats(&told), Some([(hour as u128, 1 << 2)].into()), "hour {hour}: the fast reader, the row of its step");
        if hour % 3 == 0 {
            slow.ready();
            let newest = slow.wait_newer(seen[1], LIMIT).expect("the slow reader's frame");
            seen[1] = newest.published;
            assert_eq!(newest.frame.hour, hour);
            let (_, told) = slow.take().unwrap();
            assert_eq!(told_combats(&told), Some((since as u128 + 1..=hour as u128).map(|key| (key, 1 << 2)).collect()), "hour {hour}: the slow reader, every row since its hour {since}");
            since = hour;
        }
    }
}

/// LandCombats keyed as equipment is, a division in the top of a 96-bit key: three rows each of
/// divisions 1 to 4 from hour 0, and from hour 1 the first row of division `hour % 4 + 1` moving.
fn record_divisions(name: &str, hours: u32) -> Units {
    let key = |division: u128, ordinal: u128| division << 32 | ordinal;
    record_with(name, hours, move |store, at| {
        store.status(COMBATS.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        if at == 0 {
            write(store, COMBATS, &(1..=4).flat_map(|division| (0..3).map(move |ordinal| combat(key(division, ordinal), 0))).collect::<Vec<_>>());
            return;
        }
        let mut changes = store.table(COMBATS.index()).unwrap();
        let slot = changes.slot(key(u128::from(at % 4 + 1), 0)).unwrap();
        changes.write(slot, 2, Cell::I32(at as i32)).unwrap();
    })
}

/// The keys of LandCombats' rows a frame holds.
fn combat_keys(frame: &DecodedFrame) -> Vec<u128> {
    match &*frame.tables[COMBATS.index()] {
        TableView::Complete(rows) => rows.rows::<tables::LandCombats>().unwrap().iter().map(|row| u128::from(row.id)).collect(),
        other => panic!("LandCombats holds no rows: {other:?}"),
    }
}

/// A reader of a table by key is made the rows of its keys alone, at the cost of those rows; a
/// step that moves another key's row tells it nothing; a key it newly asks is served at its next
/// frame, every row of it told as new; a row of its keys that moves is told. On the base a reader
/// took a table whole or not at all: every make of LandCombats made all twelve rows.
#[test]
fn a_reader_by_key_is_made_the_rows_of_its_keys_alone_and_a_key_newly_asked_at_its_next_frame() {
    let entry = open(record_divisions("by key", 8), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    rows.time();
    let watch = rows.reader(&[COUNTRIES], Cadence::EveryStep);
    let keyed = rows.telling(&[], Cadence::Ready);
    keyed.declare_keyed(COMBATS, Arc::new([1, 2].into()));
    rows.settled();
    rows.drive(2);
    entry.playhead().seek(2);
    at(&watch, 2);
    let first = keyed.wait_newer(0, LIMIT).expect("its first frame");
    let key = |division: u128, ordinal: u128| division << 32 | ordinal;
    let of = |divisions: &[u128]| -> Vec<u128> { divisions.iter().flat_map(|&division| (0..3).map(move |ordinal| key(division, ordinal))).collect() };
    assert_eq!(combat_keys(&first.frame), of(&[1, 2]), "the rows of its keys alone");
    assert_eq!(keyed.frame_keyed().and_then(|(frame, keys)| Arc::ptr_eq(&frame, &first.frame).then(|| keys.of(COMBATS).map(|keys| keys.iter().copied().collect::<Vec<_>>()))).flatten(), Some(vec![1, 2]));
    let _ = keyed.take();
    keyed.ready();
    rows.drive(3);
    at(&watch, 3);
    let moved_elsewhere = keyed.wait_newer(first.published, LIMIT).expect("a frame at the step that moved division 4");
    let (_, told) = keyed.take().unwrap();
    assert_eq!(told_combats(&told), Some(Default::default()), "a row of another key moved: nothing of its keys");
    keyed.declare_keyed(COMBATS, Arc::new([1, 2, 3].into()));
    keyed.ready();
    let asked = keyed.wait_newer(moved_elsewhere.published, LIMIT).expect("a frame with the key newly asked");
    assert_eq!(combat_keys(&asked.frame), of(&[1, 2, 3]));
    let (_, told) = keyed.take().unwrap();
    assert_eq!(told_combats(&told), Some(of(&[3]).into_iter().map(|key| (key, super::view::EVERY)).collect()), "every row of the key newly asked is new to it");
    keyed.ready();
    rows.drive(4);
    at(&watch, 4);
    keyed.wait_newer(asked.published, LIMIT).expect("a frame at the step that moved division 1");
    let (_, told) = keyed.take().unwrap();
    assert_eq!(told_combats(&told), Some([(key(1, 0), 1 << 2)].into()), "the row of its keys that moved");
    let timings = rows.timings().unwrap();
    let made: Vec<usize> = timings.builds[&COMBATS.index()].iter().map(|&(_, rows)| rows).collect();
    assert!(made.iter().all(|&rows| rows <= 9), "never the table's twelve rows: {made:?}");
    assert!(timings.makes[&COMBATS.index()].iter().all(|&how| how == super::Make::Keyed));
}

/// The rule for a reader that waits until it is ready, at each turn of its life: opened, it has no
/// frame and is made one at once (the seek's here); sought while it is not ready (a maker in a
/// build), it keeps the frame it has, and once it says it is ready it is made the sought hour's;
/// declared again, it has no frame and is due at once whether it said so or not. Hidden and shown
/// again, a reader is closed and opened anew, so it is the first case.
#[test]
fn a_reader_that_waits_until_ready_is_due_when_opened_sought_and_declared_again() {
    let entry = open(record_still_countries("ready at each turn", 24), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let watch = rows.reader(&[COMBATS], Cadence::EveryStep);
    let reader = rows.reader(&[COUNTRIES], Cadence::Ready);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    at(&watch, 1);
    let opened = reader.wait_newer(0, LIMIT).expect("opened: a frame at once");
    assert_eq!(opened.frame.hour, 1);
    rows.drive(10);
    entry.playhead().seek(10);
    at(&watch, 10);
    assert_eq!(reader.newest().map(|newest| (newest.frame.hour, newest.published)), Some((1, opened.published)), "sought while not ready: its frame stays");
    reader.ready();
    let sought = reader.wait_newer(opened.published, LIMIT).expect("ready: the sought hour's frame");
    assert_eq!(sought.frame.hour, 10);
    reader.declare(&[COUNTRIES, COMBATS]);
    let declared = reader.wait(|frame| !frame.tables[COMBATS.index()].is_not_declared(), LIMIT).expect("declared again: a frame at once");
    assert_eq!(declared.hour, 10);
}
