//! The map side's view store: the mirrors it reads, the sets it makes of them, and the readers it
//! makes them for. The recording builders here are the viewer's test recordings, named by the
//! other test modules that open one.

use super::{Need, Recording, Store};
#[cfg(test)]
use super::{Pace, apart::Apart};
pub use crate::playhead_host::open;
use crate::{
    layers::{Carried, Inputs, Layer, Settling, View},
    playhead_host::{
        MapSide,
        mirror::{Mirror, Mirrors},
        units::Units,
    },
};
use playhead::{
    Settings, Step, Timeline,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::{Store as ColumnarStore, Value},
    unit::{BLOCK, BlockPage, Encoder, PartSink, Settings as Writing, reader::Extents},
};
use replay_data::{
    Hour,
    rows::{Field, List, TableId, TableRows},
};
#[cfg(test)]
use replay_data::{rows::tables, scope::CaptureMode};
use replay_frame::frame::TableView;
#[cfg(test)]
use replay_frame::frame::SharedView;
#[cfg(test)]
use std::collections::BTreeSet;
use std::{io, sync::Arc, time::Duration};


pub const COUNTRIES: TableId = TableId::Countries;
pub const COMBATS: TableId = TableId::LandCombats;
pub const STRINGS: [&[u8]; 3] = [b"GER", b"ENG", b"the collector could not read it"];
pub const RECORDING: Recording = Recording { branch: 7 };
pub const LIMIT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct Memory {
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

pub struct Recorded {
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
pub enum Put {
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
pub fn country(key: u128, tag: u32, stability: i64, missing: bool) -> (u128, Vec<Put>) {
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

pub fn combat(key: u128, day: i32) -> (u128, Vec<Put>) {
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
pub struct Hourly {
    countries: Status,
    country_rows: Vec<(u128, Vec<Put>)>,
    combats: Vec<(u128, Vec<Put>)>,
}

/// The rows by hour: Countries complete with a missing optional and an unrecorded cell (hours 0
/// to 3), one row's stability moving at hour 2, truncated at 4, unavailable at 5, not recorded at
/// 6; LandCombats' day moving every hour.
pub fn hourly(hour: u32) -> Hourly {
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

pub fn write_rows(store: &mut ColumnarStore, table: TableId, rows: &[(u128, Vec<Put>)]) {
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
pub fn record(name: &str, hours: u32) -> Units {
    record_with(name, hours, |store, at| {
        let hour = hourly(at);
        store.status(COUNTRIES.index(), hour.countries, true, Some(at as i32), true).unwrap();
        if at == 0 || at == 2 {
            write_rows(store, COUNTRIES, &hour.country_rows);
        }
        store.status(COMBATS.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        write_rows(store, COMBATS, &hour.combats);
        for (table, rows) in [(TableId::CountryLinks, links()), (TableId::HumanControllers, humans())] {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
            if at == 0 {
                write_rows(store, table, &rows);
            }
        }
    })
}

/// `hours` hours from game hour 0 of every schema table, each hour's writes made by `hour`.
pub fn record_with(name: &str, hours: u32, mut hour: impl FnMut(&mut ColumnarStore, u32)) -> Units {
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut store = ColumnarStore::new(kinds.clone(), vec![false; kinds.len()]);
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
pub fn hourly_steps() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }
}

/// The rows the table's codec gives for `rows`, ascending by key, as today's frame reader hands
/// it each cell.
pub fn codec(table: TableId, rows: &[(u128, Vec<Put>)]) -> TableRows {
    let mut sorted: Vec<&(u128, Vec<Put>)> = rows.iter().collect();
    sorted.sort_by_key(|(key, _)| *key);
    let keys: Vec<u128> = sorted.iter().map(|(key, _)| *key).collect();
    table.rows_from_wire(&keys, &mut |row, column| Ok(sorted[row].1[column].field())).unwrap()
}



fn links() -> Vec<(u128, Vec<Put>)> {
    [(9, 18), (2, 4), (5, 10)].into_iter().map(|(id, slot)| (id, vec![Put::Cell(Cell::U32(slot))])).collect()
}

fn humans() -> Vec<(u128, Vec<Put>)> {
    vec![(9, vec![Put::Cell(Cell::I8(1))]), (5, vec![Put::Cell(Cell::I8(0))])]
}


/// A reader of one column of a table and of another's keys alone gets those and nothing else: a
/// read of any other field is refused by name and names what it read, so the set is widened, never
/// served a zero.
#[test]
fn a_reader_of_some_columns_or_of_keys_alone_holds_only_those_and_any_other_field_is_refused_by_name() {
    use replay_data::rows::{CountryRow, mask};
    let entry = open(record("columns", 4), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    entry.playhead().seek(3);
    let mut apart = Apart::new(&entry, RECORDING.branch).unwrap();
    let named = mask::<CountryRow>(&["stability_raw"]);
    let reader = apart.store().handle().reader_at("columns", &[Need::columns(COUNTRIES, named), Need::keys(COMBATS)], Pace::Step);
    assert!(apart.drive(3), "the store never reached hour 3");
    let frame = reader.wait(|frame| frame.hour == 3, LIMIT).expect("the rows of hour 3");
    let countries = frame.complete_fields::<tables::Countries>(named).unwrap();
    let mut sorted = hourly(3).country_rows.clone();
    sorted.sort_by_key(|(key, _)| *key);
    assert_eq!(countries.iter().map(|row| u128::from(row.id)).collect::<Vec<_>>(), sorted.iter().map(|(key, _)| *key).collect::<Vec<_>>(), "every live row by key");
    for (row, (_, cells)) in countries.iter().zip(&sorted) {
        assert!(matches!(cells[4], Put::Cell(Cell::I64(value)) if value == row.stability_raw), "the named column is the recording's");
    }
    let (whole, missed) = replay_frame::frame::discovering_fields(|| frame.complete::<tables::Countries>().map(<[_]>::len));
    assert!(whole.is_err(), "a read of every field is never served: {whole:?}");
    assert_eq!(missed, vec![(COUNTRIES, u64::MAX)]);
    let tag = mask::<CountryRow>(&["tag"]);
    let (tagged, missed) = replay_frame::frame::discovering_fields(|| frame.complete_fields::<tables::Countries>(tag | named).map(<[_]>::len));
    assert!(tagged.is_err());
    assert_eq!(missed, vec![(COUNTRIES, tag | named)], "the fields asked for, to widen the need");
    let combats = frame.complete_fields::<tables::LandCombats>(0).unwrap();
    assert_eq!(combats.len(), 2, "keys alone: both live combats");
}

/// Two readers of one table that read different columns each hold a view of their own: the narrow
/// one's fields are the whole one's, and a step that writes a column it does not read leaves its
/// rows as they were, the allocation included, so nothing it feeds is made again.
#[test]
fn a_view_of_declared_columns_holds_what_the_whole_view_holds_and_a_write_outside_it_makes_nothing() {
    use replay_data::rows::{LandCombatRow, mask};
    let (day, place) = (mask::<LandCombatRow>(&["day"]), mask::<LandCombatRow>(&["province", "duration"]));
    let column = replay_data::rows::column::<LandCombatRow>("day");
    // Every column at the first hour; after it the day alone, as a recording's change sets hold
    // the cells that moved.
    let units = record_with("column sets", 8, |store, at| {
        for (table, rows) in [(COUNTRIES, hourly(0).country_rows), (TableId::CountryLinks, links()), (TableId::HumanControllers, humans())] {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
            if at == 0 {
                write_rows(store, table, &rows);
            }
        }
        store.status(COMBATS.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        if at == 0 {
            write_rows(store, COMBATS, &hourly(0).combats);
            return;
        }
        let mut changes = store.table(COMBATS.index()).unwrap();
        for (slot, day) in [(0_u32, at as i32), (1, -(at as i32))] {
            changes.write(slot, column, Cell::I32(day)).unwrap();
        }
    });
    let entry = open(units, hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    entry.playhead().seek(1);
    let mut apart = Apart::new(&entry, RECORDING.branch).unwrap();
    let store = apart.store().handle();
    let (moving, still, whole) = (
        store.reader_at("day", &[Need::columns(COMBATS, day)], Pace::Step),
        store.reader_at("place", &[Need::columns(COMBATS, place)], Pace::Step),
        store.reader_at("whole", &[Need::every(COMBATS)], Pace::Step),
    );
    let mut held: Option<SharedView> = None;
    for hour in 1..7 {
        assert!(apart.drive(hour), "the store never reached hour {hour}");
        let at = |reader: &super::Reader| reader.wait(|frame| frame.hour == hour, LIMIT).unwrap_or_else(|error| panic!("hour {hour}: {error}"));
        let (moved, stood, every) = (at(&moving), at(&still), at(&whole));
        let all = every.complete::<tables::LandCombats>().unwrap();
        let days = moved.complete_fields::<tables::LandCombats>(day).unwrap();
        let places = stood.complete_fields::<tables::LandCombats>(place).unwrap();
        assert_eq!(days.iter().map(|row| (row.id, row.day)).collect::<Vec<_>>(), all.iter().map(|row| (row.id, row.day)).collect::<Vec<_>>(), "hour {hour}: the day column is the whole view's");
        assert_eq!(places.iter().map(|row| (row.id, row.province, row.duration)).collect::<Vec<_>>(), all.iter().map(|row| (row.id, row.province, row.duration)).collect::<Vec<_>>(), "hour {hour}: the place columns are the whole view's");
        let view = stood.tables[COMBATS.index()].clone();
        if let Some(before) = held.replace(view.clone()) {
            assert!(SharedView::ptr_eq(&before, &view), "hour {hour}: only the day was written, so the place columns' rows are the same allocation");
        }
        assert!(!SharedView::ptr_eq(&moved.tables[COMBATS.index()], &view), "hour {hour}: each set of columns has rows of its own");
    }
}

/// A table the recording holds nothing of comes as not recorded, never as a zero and never as a
/// frame withheld for ever: the side's mirrors hold every table of the schema for that.
#[test]
fn a_table_the_recording_holds_nothing_of_comes_as_not_recorded_and_withholds_no_frame() {
    let entry = open(record("never held", 4), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    entry.playhead().seek(1);
    let mut apart = Apart::new(&entry, RECORDING.branch).unwrap();
    let reader = apart.store().handle().reader_at("absent", &[Need::every(COMBATS), Need::every(TableId::Divisions)], Pace::Step);
    assert!(apart.drive(1), "the store never reached hour 1");
    let frame = reader.wait(|frame| frame.hour == 1, LIMIT).expect("the rows of hour 1");
    assert_eq!(*frame.tables[TableId::Divisions.index()], TableView::NotRecorded { mode: CaptureMode::Full }, "refused by name, not waited for");
    assert_eq!(*frame.tables[COMBATS.index()], TableView::Complete(codec(COMBATS, &hourly(1).combats)));
    assert!(reader.arriving().is_empty(), "nothing it declared is still awaited: {:?}", reader.arriving());
}

/// The table the layer itself reads.
const DRAWN: TableId = TableId::Countries;

/// The table only its maker reads, which the side's mirrors hold for the store alone.
const MADE: TableId = TableId::LandCombats;

/// A table the maker reads by key: the rows of the keys it asks for, and no other.
const KEYED: TableId = TableId::CountryLinks;

/// A table no layer and no maker reads until one learns it mid-play: the side mirrors none of it
/// before that, so learning it is what brings it.
const LEARNED: TableId = TableId::HumanControllers;

/// A layer that reads one table itself and whose maker reads another from the side's view store:
/// at every settle it keeps the hour the mirrors stand at and what the frame its maker was given
/// holds — its branch, its hour and the rows of the table only the maker reads.
struct Maker {
    reader: Option<super::Reader>,
    composes: u32,
    seen: Vec<(Option<Hour>, Option<(u64, Hour, usize)>)>,
    /// Set by a test: at the next settle the maker declares [`LEARNED`] beside its own table, as
    /// a builder that has just read a table it did not declare does.
    learn: bool,
}

impl Maker {
    fn new() -> Self {
        Self { reader: None, composes: 0, seen: Vec::new(), learn: false }
    }
}

impl Layer for Maker {
    fn name(&self) -> &'static str {
        "maker"
    }

    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        inputs.table(DRAWN, true)?;
        inputs.index(DRAWN, 0)
    }

    fn views(&mut self, store: &Store) -> io::Result<()> {
        self.reader = Some(store.reader("test", &[Need::every(MADE)]));
        Ok(())
    }

    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        self.composes += 1;
        Ok(())
    }

    fn apply(&mut self, _step: &Step, _mirrors: &Mirrors) {}

    fn settle(&mut self, mirrors: &Mirrors, _settling: Settling) -> io::Result<()> {
        let frame = self.reader.as_ref().and_then(super::Reader::frame).map(|frame| {
            let rows = match &*frame.tables[MADE.index()] {
                TableView::Complete(rows) => rows.len(),
                _ => usize::MAX,
            };
            (frame.branch, frame.hour, rows)
        });
        self.seen.push((mirrors.get(DRAWN).and_then(Mirror::head), frame));
        if let Some(reader) = &self.reader
            && self.learn
        {
            self.learn = false;
            reader.declare_only(&[MADE, LEARNED]);
        }
        Ok(())
    }

    fn carried(&self) -> Carried {
        Carried::default()
    }

    fn held(&self) -> usize {
        0
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }

    fn any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// Whether the frame the maker read at its last settle stood at the hour the side had applied;
/// false while the side's parts are away. What the frame a settle sees stands at is what a test of
/// the store asserts on, so a frame made after that settle is not enough.
fn at_hour(side: &MapSide, hour: Hour) -> bool {
    let _ = hour;
    side.parts().is_some() && maker(side).seen.last().is_some_and(|&(applied, frame)| matches!((applied, frame), (Some(applied), Some((_, hour, _))) if hour == applied))
}

fn maker(side: &MapSide) -> &Maker {
    side.parts().unwrap().1[0].any().downcast_ref::<Maker>().unwrap()
}

/// Every frame the maker was given while the side stepped: the hour the mirrors stood at, and the
/// frame's own branch, hour and rows.
fn answered(side: &MapSide) -> Vec<(Hour, (u64, Hour, usize))> {
    maker(side).seen.iter().filter_map(|&(applied, frame)| applied.zip(frame)).collect()
}

/// Frames until the side has applied `hour`, is not composing, and its maker has been given the
/// frame of the hour it applied. A frame is withheld while a table the maker declared is not home
/// yet, so the hour applied can be a step ahead of the frame for a step or two.
fn until(side: &mut MapSide, hour: Hour) {
    for _ in 0..200_000u64 {
        let head = side.entry().playhead().newest().unwrap_or(0);
        side.frame(head).unwrap();
        let applied = side.applied();
        let stood = applied.is_some_and(|applied| at_hour(side, applied));
        if applied.is_some_and(|applied| applied >= hour) && !side.composing() && stood {
            return;
        }
        std::thread::yield_now();
    }
    panic!("the map side never applied hour {hour} with its maker at that hour");
}

fn side_of(name: &str, hours: u32) -> (Arc<crate::playhead_host::Entry>, MapSide) {
    let entry = open(record(name, hours), hourly_steps()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Maker::new())], View::default()).unwrap();
    side.set_branch(RECORDING.branch);
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    (entry, side)
}

/// The store makes its views from the side's own mirrors inside the step, so a frame a maker is
/// given stands at the hour the side applied — never at an older one, which is what the rows
/// worker's cadence left it with — and it holds the rows of a table no drawn layer reads.
#[test]
fn a_maker_is_given_the_rows_of_the_hour_the_side_applied() {
    let (_entry, mut side) = side_of("views at the hour", 12);
    until(&mut side, 0);
    // A step a frame (four hours), however far the playhead has built.
    for _ in 0..200_000u64 {
        if side.applied().is_some_and(|applied| applied >= 6) && !side.composing() && at_hour(&side, side.applied().unwrap()) {
            break;
        }
        let head = side.entry().playhead().newest().unwrap_or(0).min(side.applied().unwrap_or(0) + 4);
        side.frame(head).unwrap();
        std::thread::yield_now();
    }
    // A machine held up long enough lets the playing playhead run past the steps between, so the
    // maker may be given one frame only, at the end.
    let answers = answered(&side);
    assert!(!answers.is_empty(), "the maker was given a frame while the side stepped: {:?}", maker(&side).seen);
    for &(applied, (branch, hour, rows)) in &answers {
        assert!(hour <= applied, "a view is never made from mirrors past the hour applied: applied {applied}, frame {hour}");
        assert_eq!(branch, RECORDING.branch, "the frame carries the recording's branch");
        assert_ne!(rows, usize::MAX, "{MADE:?} is complete in the frame of hour {hour}");
    }
    let (applied, (_, hour, rows)) = *answers.last().expect("a frame");
    assert_eq!(hour, applied, "the last frame stands at the hour the side applied");
    assert!(rows > 0, "it holds the rows of the table only the maker reads");
    assert!(answers.iter().any(|&(applied, (_, hour, _))| hour == applied), "a frame of the hour applied");
}

/// The mirrors travel to the compose worker with a seek's reset: while they are away no view is
/// made, and the first step after the swap makes every set whole from the mirrors rebuilt, so the
/// maker reads the hour sought and not the hour before it.
#[test]
fn a_maker_reads_the_rows_again_after_a_seeks_compose() {
    let (entry, mut side) = side_of("views through a compose", 24);
    until(&mut side, 2);
    let before = answered(&side).len();
    entry.playhead().seek(18);
    until(&mut side, 18);
    let answers = answered(&side);
    assert!(answers.len() > before, "it was given a frame after the compose");
    let (applied, (_, hour, rows)) = *answers.last().expect("a frame");
    assert_eq!(hour, applied, "the frame after a compose stands at the hour applied");
    assert!(applied >= 18, "the side applied the hour sought: {applied}");
    assert!(rows > 0, "its rows are made whole from the mirrors rebuilt");
}


/// A maker asked for the rows of other keys is answered from the mirrors the side's last step
/// left, without waiting for a step that may never come: the frame it is given next stands at the
/// hour the side stands at and holds those keys' rows.
#[test]
fn a_maker_asking_for_other_keys_is_answered_at_the_hour_the_side_stands_at() {
    let (_entry, mut side) = side_of("views keyed at a held hour", 12);
    until(&mut side, 2);
    // The table read by key, declared and its mirror waited for: the side holds it before the
    // test asks for the rows of any key.
    maker(&side).reader.as_ref().expect("its reader").declare_keyed(KEYED, Arc::new(BTreeSet::new()));
    let last = side.entry().playhead().newest().expect("the recording's hours");
    until(&mut side, last);
    for _ in 0..200_000u64 {
        side.frame(last).unwrap();
        if side.parts().is_some()
            && maker(&side).reader.as_ref().and_then(super::Reader::frame).is_some_and(|frame| !frame.tables[KEYED.index()].is_not_declared())
        {
            break;
        }
        std::thread::yield_now();
    }
    // Every hour of the recording applied: no later step moves a set, so only the keys asked for
    // can bring the maker a frame.
    let reader = maker(&side).reader.as_ref().expect("its reader");
    let before = reader.frame().expect("a frame").hour;
    let keys: BTreeSet<u64> = (1..=2).collect();
    reader.declare_keyed(KEYED, Arc::new(keys.clone()));
    let (frame, held) = reader.frame_keyed().expect("a frame made for the keys asked");
    assert_eq!(frame.hour, before, "it is answered at the hour the side stands at, with no step between");
    assert_eq!(held.of(KEYED).map(|held| held.iter().copied().collect::<Vec<u64>>()), Some(keys.into_iter().collect::<Vec<u64>>()), "its frame's rows of {KEYED:?} were made for the keys it asked for");
    assert!(matches!(&*frame.tables[KEYED.index()], TableView::Complete(_)), "{KEYED:?} is complete in it: {:?}", frame.tables[KEYED.index()].held());
}

/// A maker that reads some columns of a table keeps that set when it declares a table by key: the
/// rows it is given hold those columns and refuse any other by name.
#[test]
fn a_maker_keeps_its_columns_when_it_reads_a_table_by_key() {
    let entry = open(record("views keyed columns", 4), hourly_steps()).unwrap();
    entry.playhead().seek(0);
    let mut fed = super::apart::Apart::new(&entry, RECORDING.branch).unwrap();
    // Two of the table's columns, by their place in its schema.
    let columns: u64 = 0b101;
    let reader = fed.store().reader("test", &[Need::columns(MADE, columns)]);
    reader.declare_keyed(KEYED, Arc::new((1..=2).collect()));
    assert!(fed.drive(2), "the store never reached hour 2");
    let frame = reader.frame().expect("a frame of the hour driven to");
    assert_eq!(frame.tables[MADE.index()].fields(), Some(columns), "its rows of {MADE:?} hold the columns it named and no other");
    assert!(!frame.tables[KEYED.index()].is_not_declared(), "the table it reads by key is in its frame");
}

/// A maker that asks for other keys during a post at the hour it already stands at, after the
/// post chose the readers to publish, is answered by that post: its frame holds the rows of the
/// keys it asked for with no step after. The store's test step ([`super::Shared::chosen`]) asks
/// between the choice and the jobs posted. On `303632584d` the mirrors such a maker is answered
/// from were set only after that point, so it asked into none and kept a frame without those rows
/// until another post.
#[test]
fn a_maker_asking_for_other_keys_during_a_same_hour_post_is_answered_by_that_post() {
    let entry = open(record("views keyed mid post", 4), hourly_steps()).unwrap();
    entry.playhead().seek(0);
    let mut apart = Apart::new(&entry, RECORDING.branch).unwrap();
    let reader = Arc::new(apart.store().reader("test", &[Need::every(MADE)]));
    reader.declare_keyed(KEYED, Arc::new(BTreeSet::new()));
    assert!(apart.drive(3), "the store never reached hour 3");
    let (frame, held) = reader.frame_keyed().expect("a frame of hour 3");
    assert_eq!(frame.hour, 3);
    assert_eq!(held.of(KEYED).map(|held| held.len()), Some(0), "no keys asked yet");
    let keys: BTreeSet<u64> = links().iter().map(|&(key, _)| super::prefix(key)).collect();
    *apart.store().shared.chosen.lock().unwrap() = Some(Box::new({
        let (reader, keys) = (Arc::clone(&reader), keys.clone());
        move || reader.declare_keyed(KEYED, Arc::new(keys))
    }));
    assert!(apart.drive(3), "the store posted hour 3 again");
    assert!(apart.store().shared.chosen.lock().unwrap().is_none(), "the post asked for the keys");
    let (frame, held) = reader.frame_keyed().expect("a frame");
    assert_eq!(frame.hour, 3, "answered at the hour the store stands at");
    assert_eq!(held.of(KEYED).map(|held| held.iter().copied().collect::<Vec<u64>>()), Some(keys.into_iter().collect::<Vec<u64>>()), "its frame's rows of {KEYED:?} were made for the keys it asked for");
    match &*frame.tables[KEYED.index()] {
        TableView::Complete(rows) => assert_eq!(rows.len(), links().len(), "every country link's row"),
        other => panic!("{KEYED:?} is complete in its frame: {other:?}"),
    }
}

/// Whether the maker's frame holds [`LEARNED`] complete, and the hour it stands at; none while
/// the frame does not declare it, which is what a maker that has not learned it has.
fn learned_at(side: &MapSide) -> Option<(Hour, usize)> {
    let frame = maker(side).reader.as_ref().and_then(super::Reader::frame)?;
    if frame.tables[LEARNED.index()].is_not_declared() {
        return None;
    }
    match &*frame.tables[LEARNED.index()] {
        TableView::Complete(rows) => Some((frame.hour, rows.len())),
        _ => None,
    }
}

/// Frames at `head` until the maker's frame holds [`LEARNED`], or a panic naming what it waited
/// for. `head` is the frames' own head: a stepped frame gives the hour it holds, never the newest.
fn until_learned(side: &mut MapSide, head: Option<Hour>, why: &str) -> (Hour, usize) {
    for _ in 0..200_000u64 {
        let at = head.unwrap_or_else(|| side.entry().playhead().newest().unwrap_or(0));
        side.frame(at).unwrap();
        if let Some(held) = learned_at(side) {
            return held;
        }
        std::thread::yield_now();
    }
    let arriving = maker(side).reader.as_ref().map(super::Reader::arriving);
    let mirror = side.parts().map(|(mirrors, _)| mirrors.get(LEARNED).is_some());
    panic!(
        "the maker never read {LEARNED:?} {why}: arriving {arriving:?}, mirror {mirror:?}, applied {:?}, newest {:?}, head {head:?}",
        side.applied(),
        side.entry().playhead().newest()
    );
}

/// A table no layer and no maker reads is mirrored by nobody, so no change set of it is applied;
/// a maker that learns it mid-play is given its rows, at the hour the side applies, and never a
/// frame that holds it as not declared while it reads it.
#[test]
fn a_table_learned_while_playing_reads_at_the_hour_the_side_applied() {
    let (_entry, mut side) = side_of("views learned playing", 24);
    until(&mut side, 4);
    assert!(learned_at(&side).is_none(), "{LEARNED:?} is in no frame before a maker reads it");
    side.layers_mut()[0].any_mut().and_then(|layer| layer.downcast_mut::<Maker>()).expect("the maker").learn = true;
    let (hour, rows) = until_learned(&mut side, None, "while the side played");
    assert!(rows > 0, "the rows of {LEARNED:?} it learned");
    let applied = side.applied().expect("the side applied an hour");
    assert!(hour <= applied, "the frame holding {LEARNED:?} stands at an hour the side applied: {hour} against {applied}");
    // Two hours on, or the recording's last (hour 23) when it was learned near its end.
    until(&mut side, (hour + 2).min(23));
    let (after, _) = learned_at(&side).expect("it keeps reading what it learned");
    assert_eq!(Some(after), side.applied(), "once learned it stands at the hour the side applies, like every other table it reads");
}

/// The same while a stepped frame holds the head: the side's head does not move, so a table
/// learned must come to it where it stands. On the lane's base every table was mirrored from the
/// first declaration, which is what this replaces.
#[test]
fn a_table_learned_while_a_stepped_frame_holds_the_head_reads_at_that_head() {
    let (entry, mut side) = side_of("views learned held", 24);
    until(&mut side, 6);
    entry.playhead().play(0.0);
    let held = side.applied().expect("the side applied an hour");
    side.layers_mut()[0].any_mut().and_then(|layer| layer.downcast_mut::<Maker>()).expect("the maker").learn = true;
    let (hour, rows) = until_learned(&mut side, Some(held), "while a stepped frame held the head");
    assert!(rows > 0, "the rows of {LEARNED:?} it learned");
    assert_eq!(hour, held, "it reads {LEARNED:?} at the hour the stepped frame holds, never at a newer one");
}

/// A post whose mirrors stand short of its hour (a table a step left stalled for a part not read
/// yet) gives no reader a frame stamped with that hour: the rows of hour 3 are never said to be
/// hour 4's. Before, the store checked only that the mirror was home.
#[test]
fn a_mirror_short_of_the_posted_hour_makes_no_frame_of_that_hour() {
    let entry = open(record("stalled short", 8), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    entry.playhead().seek(2);
    let mut apart = Apart::new(&entry, RECORDING.branch).unwrap();
    let reader = apart.store().handle().reader_at("short", &[Need::every(COMBATS)], Pace::Step);
    assert!(apart.drive(3), "the store never reached hour 3");
    reader.wait(|frame| frame.hour == 3, LIMIT).unwrap_or_else(|error| panic!("the rows of hour 3: {error}"));
    apart.post_at(4);
    assert_eq!(reader.frame().map(|frame| frame.hour), Some(3), "no frame of hour 4 from mirrors standing at 3");
}

/// A reader of the display frame's own keeps the frames before its newest: the side prepares its
/// rows ahead of the hour on screen, so the display reads the newest frame no later than that
/// hour, and none for an hour before every frame it was given, never rows past it. A frame kept
/// from before a seek is another history's: after the seek to 7, hour 5 has none.
#[test]
fn a_display_frames_reader_gives_the_newest_frame_no_later_than_the_hour_shown() {
    let entry = open(record("display through", 8), hourly_steps()).unwrap();
    entry.set_capture(CaptureMode::Full);
    entry.playhead().seek(2);
    let mut apart = Apart::new(&entry, RECORDING.branch).unwrap();
    let reader = apart.store().handle().reader_later("display", &[Need::every(COMBATS)], Pace::Step);
    for hour in [2, 3, 5] {
        assert!(apart.drive(hour), "the store never reached hour {hour}");
        reader.wait(|frame| frame.hour == hour, LIMIT).unwrap_or_else(|error| panic!("the rows of hour {hour}: {error}"));
    }
    let through = |hour: Hour| reader.frame_now_through(hour).map(|frame| frame.hour);
    assert_eq!(reader.frame_now().map(|frame| frame.hour), Some(5), "its newest frame is past the hour shown");
    assert_eq!(through(5), Some(5));
    // The store may also post hour 4 on its way to 5: the frame is 3's or 4's, never 5's.
    let between = through(4);
    assert!(between.is_some_and(|hour| (3..=4).contains(&hour)), "the newest frame no later than the hour shown, older than the newest: {between:?}");
    assert_eq!(through(2), Some(2));
    assert_eq!(through(1), None, "no frame for an hour before every frame it was given");
    entry.playhead().seek(7);
    assert!(apart.drive(7), "the store never reached hour 7 after the seek");
    reader.wait(|frame| frame.hour == 7, LIMIT).unwrap_or_else(|error| panic!("the rows of hour 7: {error}"));
    assert_eq!(through(7), Some(7));
    assert_eq!(through(5), None, "the frame of hour 5 is of the history before the seek");
}
