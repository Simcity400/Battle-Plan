use super::HeadRows;
use crate::playhead_host::{
    open,
    units::Units,
    views::{Recording, Views, apart::Apart},
};
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
    rows::{ClockRow, DivisionRow, Field, List, Opaque, RowCodec, SimulationFeatureRow, TableId},
    scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeSet, io, sync::Arc};
use viewer_native_gui::counter_hovers::unit_hits::{Hit, Kind as HitKind};
use viewer_unit_reads::counter_move::MovementFrame;
use viewer_runtime::unit_handle::Unit;

const RECORDING: Recording = Recording { branch: 3 };
const STRINGS: [&[u8]; 1] = [b""];
const SPEED: i32 = 3;

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

fn put<R: RowCodec>(store: &mut Store, table: TableId, slot: u32, row: &R) {
    let mut changes = store.table(table.index()).unwrap();
    if changes.slot(row.key()).is_none() {
        changes.insert_at(slot, row.key()).unwrap();
    }
    row.visit_wire(&mut |column, field| {
        let cell = match field {
            Field::U8(value) => Cell::U8(value),
            Field::I8(value) => Cell::I8(value),
            Field::U16(value) => Cell::U16(value),
            Field::I16(value) => Cell::I16(value),
            Field::U32(value) => Cell::U32(value),
            Field::I32(value) => Cell::I32(value),
            Field::U64(value) => Cell::U64(value),
            Field::I64(value) => Cell::I64(value),
            Field::Bool(value) => Cell::Bool(value),
            Field::OptionalI32(value) => Cell::OptionalI32(value),
            Field::OptionalU32(value) => Cell::OptionalU32(value),
            Field::OptionalI64(value) => Cell::OptionalI64(value),
            Field::OptionalU64(value) => Cell::OptionalU64(value),
            Field::Bytes(bytes) => Cell::Bytes(STRINGS.iter().position(|text| *text == bytes).expect("a default row's text is empty") as u32),
            Field::U32List(List::Native(items)) => Cell::U32List(items),
            Field::U64List(List::Native(items)) => Cell::U64List(items),
            Field::Opaque(Opaque::Encoded(bytes, _)) => Cell::Opaque(bytes),
            _ => return changes.put_value(slot, column, Value::Unrecorded),
        };
        changes.write(slot, column, cell)
    })
    .unwrap();
}

fn record(name: &str, hours: u32) -> Units {
    let kinds = replay_recording::schema::schema().unwrap();
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing::at(3)).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..hours {
        store.begin_hour(at as i32, if at == 0 { &STRINGS[..] } else { &[] }).unwrap();
        for table in [TableId::Divisions, TableId::Clock, TableId::SimulationFeatures] {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
        if at == 0 {
            put(&mut store, TableId::Divisions, 0, &DivisionRow { id: 1, ..Default::default() });
            put(&mut store, TableId::Clock, 0, &ClockRow { speed: SPEED, ..Default::default() });
            put(&mut store, TableId::SimulationFeatures, 0, &SimulationFeatureRow::default());
        }
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
    Units { name: format!("head rows test {name}"), pages, timeline, kinds }
}

fn hourly() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }
}

fn nothing(hour: Hour) -> DecodedFrame {
    DecodedFrame { branch: RECORDING.branch, hour, tables: std::array::from_fn(|_| replay_frame::frame::not_selected()), metrics: Default::default() }
}

fn opened(name: &str) -> (Apart, Views) {
    let entry = open(record(name, 40), hourly()).unwrap();
    entry.set_capture(CaptureMode::Full);
    entry.playhead().seek(1);
    let mut fed = Apart::new(&entry, RECORDING.branch).unwrap();
    assert!(fed.drive(1), "the store never reached hour 1");
    let views = fed.store().handle();
    (fed, views)
}

fn step(fed: &mut Apart, hour: Hour) {
    assert!(fed.drive(hour), "the store never reached hour {hour}");
}

fn held(fed: &Apart) -> Vec<TableId> {
    fed.store().held().iter().map(|(table, _)| *table).collect()
}

#[test]
fn the_unit_selection_observer_reads_rows_at_the_head_and_holds_nothing_with_nothing_selected() {
    let (mut fed, views) = opened("units");
    let head = Some(&views);
    let today = nothing(1);
    let mut head_rows = HeadRows::default();
    let both = || {
        let mut selection = viewer_counters::unit_selection::Selection::default();
        selection.context(RECORDING.branch);
        let hit = Hit { ours: true, port: false, rect: [0.0; 4], kind: HitKind::Division, members: [1, 2].into(), layer: 0, plate: None };
        selection.click(Some(&hit), false);
        selection
    };
    let observe = |head_rows: &mut HeadRows, head| {
        let mut selection = both();
        head_rows.units(head, Some(&today), |frame| selection.observe(RECORDING.branch, frame));
        selection.units().iter().copied().collect::<BTreeSet<Unit>>()
    };
    let all: BTreeSet<Unit> = [Unit::Division(1), Unit::Division(2)].into();
    assert_eq!(observe(&mut head_rows, None), all, "off: today's frame holds no census, so nothing is dropped");
    assert_eq!(observe(&mut head_rows, head), all, "nothing read while its rows load: the selection keeps what it holds");
    let mut seen = None;
    for hour in 2..38 {
        head_rows.end_frame(None);
        step(&mut fed, hour);
        let read = observe(&mut head_rows, head);
        if read != all {
            seen = Some(read);
            break;
        }
    }
    assert_eq!(seen, Some([Unit::Division(1)].into()), "at the head only division 1 lives");
    assert!(held(&fed).contains(&TableId::RailwayGunCounterInputs), "the observer's tables are held while it is asked");
    head_rows.end_frame(None);
    head_rows.end_frame(None);
    step(&mut fed, 39);
    assert!(!held(&fed).contains(&TableId::RailwayGunCounterInputs), "a frame that did not ask retires its tables");
}

#[test]
fn the_clock_speed_and_the_pickers_feature_read_rows_at_the_head() {
    let (mut fed, views) = opened("map");
    let head = Some(&views);
    let today = nothing(1);
    let mut head_rows = HeadRows::default();
    let read = |head_rows: &mut HeadRows, head| (head_rows.map(head, Some(&today), MovementFrame::clock_speed), head_rows.map(head, Some(&today), crate::app::picker::four_buttons_at));
    assert_eq!(read(&mut head_rows, None), (Some(None), Some(true)), "off: today's frame records neither");
    assert_ne!(read(&mut head_rows, head), (Some(None), Some(true)), "never today's");
    let mut at_head = None;
    for hour in 2..38 {
        head_rows.end_frame(None);
        step(&mut fed, hour);
        if let (Some(Some(speed)), Some(four)) = read(&mut head_rows, head) {
            at_head = Some((speed, four));
            break;
        }
    }
    assert_eq!(at_head, Some((SPEED, false)), "the recorded speed, and features without the fourth button's");
}

#[test]
fn a_click_on_an_hour_never_read_is_answered_over_that_hours_rows_once_they_arrive() {
    let (mut fed, views) = opened("clicks");
    let head = Some(&views);
    let mut head_rows = HeadRows::default();
    head_rows.click_at(Some((RECORDING.branch, 1)));
    let click = |head_rows: &mut HeadRows| head_rows.clicks(head, None, |frame| (frame.hour, frame.complete::<replay_data::rows::tables::Divisions>().map(|rows| rows.len()).ok()));
    assert_eq!(click(&mut head_rows), None, "the store has made nothing of hour 1 for the click yet");
    head_rows.end_frame(head);
    head_rows.end_frame(head);
    step(&mut fed, 1);
    step(&mut fed, 2);
    assert_eq!(click(&mut head_rows), Some((1, Some(1))), "answered over the rows of the hour it was made at, not the hour after");
}

#[test]
fn the_click_reader_makes_no_family_the_view_cannot_click() {
    use super::{Clickable, click_tables, hidden_families};
    let hidden = hidden_families(Clickable::GROUND.shown);
    assert!(hidden.contains(&TableId::DivisionStatus) && hidden.contains(&TableId::AirWings) && hidden.contains(&TableId::NavalMissionLedgers));
    let every = Clickable { shown: viewer_counters::counters::Shown::EVERY, air: true, divisions: true, raids: true };
    let mut learning = HeadRows::default();
    learning.clickable(every);
    learning.learned = vec![TableId::DivisionStatus, TableId::Characters];
    assert!(click_tables(every, &learning.learned).contains(&TableId::DivisionStatus), "a learned family table while its family can be clicked");
    learning.clickable(Clickable::GROUND);
    let ground = click_tables(Clickable::GROUND, &learning.learned);
    assert_eq!(ground.iter().filter(|table| hidden.contains(table)).collect::<Vec<_>>(), Vec::<&TableId>::new(), "no hidden family's table");
    assert!(ground.contains(&TableId::Provinces) && ground.contains(&TableId::Characters), "the ground's tables, and a learned table no family names");

    let (mut fed, views) = opened("families");
    let head = Some(&views);
    let mut head_rows = HeadRows::default();
    head_rows.click_at(Some((RECORDING.branch, 1)));
    head_rows.clicks(head, None, |frame| frame.hour);
    head_rows.end_frame(head);
    step(&mut fed, 2);
    let on_ground = held(&fed);
    assert!(on_ground.contains(&TableId::Provinces), "the click reader holds the ground's tables: {on_ground:?}");
    assert_eq!(on_ground.iter().filter(|table| hidden.contains(table)).collect::<Vec<_>>(), Vec::<&TableId>::new(), "and no hidden family's");
    head_rows.clickable(Clickable { divisions: true, ..Clickable::GROUND });
    head_rows.end_frame(head);
    step(&mut fed, 3);
    assert!(held(&fed).contains(&TableId::ArmyIconInputs), "divisions that can be clicked: their sounds' tables");
    head_rows.clickable(Clickable::GROUND);
    head_rows.end_frame(head);
    step(&mut fed, 4);
    assert!(!held(&fed).contains(&TableId::ArmyIconInputs), "let go once they cannot");
}

#[test]
fn a_click_reading_a_hidden_familys_table_is_answered_and_the_table_goes_with_the_view() {
    use super::{Clickable, hidden_families};
    let raids = Clickable { raids: true, ..Clickable::GROUND };
    assert!(hidden_families(raids.shown).contains(&TableId::ModifierDefinitions));
    let (mut fed, views) = opened("hidden family");
    let head = Some(&views);
    let mut head_rows = HeadRows::default();
    head_rows.clickable(raids);
    let read = |frame: &DecodedFrame| frame.complete::<replay_data::rows::tables::ModifierDefinitions>().map(|rows| rows.len()).ok();
    head_rows.click_at(Some((RECORDING.branch, 1)));
    let mut answered = head_rows.clicks(head, None, read);
    for hour in 2..38 {
        if answered.is_some() {
            break;
        }
        head_rows.end_frame(head);
        step(&mut fed, hour);
        head_rows.click_at(Some((RECORDING.branch, hour)));
        answered = head_rows.clicks(head, None, read);
    }
    assert!(answered.is_some(), "the click is answered once the table it read is held");
    assert!(held(&fed).contains(&TableId::ModifierDefinitions), "held while the view stands");
    head_rows.clickable(Clickable::GROUND);
    head_rows.end_frame(head);
    step(&mut fed, 39);
    assert!(!held(&fed).contains(&TableId::ModifierDefinitions), "let go once the view's families change");
}

#[test]
fn readers_at_the_head_answer_no_later_than_the_hour_the_display_shows() {
    let (mut fed, views) = opened("through");
    let head = Some(&views);
    let mut head_rows = HeadRows::default();
    head_rows.click_at(Some((RECORDING.branch, 4)));
    let ask = |head_rows: &mut HeadRows| (head_rows.map(head, None, |frame| frame.hour), head_rows.clicks(head, None, |frame| frame.hour));
    for hour in 2..=6 {
        ask(&mut head_rows);
        head_rows.end_frame(head);
        step(&mut fed, hour);
    }
    assert_eq!(ask(&mut head_rows).0, Some(6), "unbounded, the newest rows");
    views.show_through(Some(4));
    let (clock, click) = ask(&mut head_rows);
    assert!(clock.is_some_and(|hour| (3..=4).contains(&hour)), "the clock's reader at {clock:?}");
    assert_eq!(click, Some(4), "the click's reader at the hour it was made");
}
