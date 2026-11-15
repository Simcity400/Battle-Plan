use super::{
    HeadRows, OverlayInputs,
    tables::{Job, Outcome},
};
use crate::{
    document::OverlayRows,
    playhead_host::{
        Entry, open,
        rows::{Cadence, Recording, RowReader, Rows},
        units::Units,
    },
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
    rows::{ClockRow, DivisionRow, Field, List, Opaque, RowCodec, SimulationFeatureRow, TableId, tables},
    scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::BTreeSet,
    io,
    sync::Arc,
    time::{Duration, Instant},
};
use viewer_foundation::{
    map::{
        counter_hovers::unit_hits::{Hit, Kind as HitKind},
        counter_move::MovementFrame,
        mode_visibility::PriorityTable,
        perspective::Country,
    },
    unit_handle::Unit,
};

const RECORDING: Recording = Recording { branch: 3 };
const LIMIT: Duration = Duration::from_secs(10);
/// The only string the rows name: every text field of a default row is empty.
const STRINGS: [&[u8]; 1] = [b""];
const SPEED: i32 = 3;
/// A saved frame's settle: the window's own limit.
const SETTLE: Duration = Duration::from_secs(120);

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

/// `row` written as its codec puts it on the wire, at the first free slot.
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

/// `hours` hours from game hour 0: division 1 and the clock and features rows every hour.
fn record(name: &str, hours: u32) -> Units {
    let kinds = replay::columnar_frames::schema().unwrap();
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
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0 }
}

/// A frame of no table, as today's frame reader holds an hour it has not decoded.
fn nothing(hour: Hour) -> DecodedFrame {
    DecodedFrame { branch: RECORDING.branch, hour, tables: std::array::from_fn(|_| replay_frame::frame::not_selected()), metrics: Default::default() }
}

/// A playhead at hour 1, its rows open, and a reader of `watch` to step them by.
fn opened(name: &str) -> (Arc<Entry>, Arc<Rows>, RowReader) {
    let entry = open(record(name, 40), hourly()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let watch = rows.reader(&[TableId::Divisions], Cadence::EveryStep);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    watch.wait(|frame| frame.hour == 1, LIMIT).unwrap();
    (entry, rows, watch)
}

/// Steps the head an hour; a table declared while held joins a step past it.
fn step(rows: &Rows, watch: &RowReader, hour: Hour) {
    rows.drive(hour);
    watch.wait(|frame| frame.hour == hour, LIMIT).unwrap();
}

fn held(rows: &Rows) -> Vec<TableId> {
    rows.held().mirrors.iter().map(|(table, _)| *table).collect()
}

#[test]
fn the_unit_selection_observer_reads_rows_at_the_head_and_holds_nothing_with_nothing_selected() {
    let (entry, rows, watch) = opened("units");
    let head = Some((&entry, RECORDING));
    let today = nothing(1);
    let mut head_rows = HeadRows::default();
    let both = || {
        let mut selection = viewer_counters::map::unit_selection::Selection::default();
        selection.context(RECORDING.branch, &Country::Recorded, 0);
        let hit = Hit { ours: true, port: false, rect: [0.0; 4], kind: HitKind::Division, members: [1, 2].into(), layer: 0, plate: None };
        selection.click(Some(&hit), false);
        selection
    };
    let observe = |head_rows: &mut HeadRows, head| {
        let mut selection = both();
        head_rows.units(head, Some(&today), |frame| selection.observe(RECORDING.branch, &Country::Recorded, 0, frame));
        selection.units().iter().copied().collect::<BTreeSet<Unit>>()
    };
    let all: BTreeSet<Unit> = [Unit::Division(1), Unit::Division(2)].into();
    assert_eq!(observe(&mut head_rows, None), all, "off: today's frame holds no census, so nothing is dropped");
    assert_eq!(observe(&mut head_rows, head), all, "nothing read while its rows load: the selection keeps what it holds");
    let mut seen = None;
    for hour in 2..38 {
        head_rows.end_frame();
        step(&rows, &watch, hour);
        let read = observe(&mut head_rows, head);
        if read != all {
            seen = Some(read);
            break;
        }
    }
    assert_eq!(seen, Some([Unit::Division(1)].into()), "at the head only division 1 lives");
    assert!(held(&rows).contains(&TableId::RailwayGunCounterInputs), "the observer's tables are held while it is asked");
    head_rows.end_frame();
    head_rows.end_frame();
    step(&rows, &watch, 39);
    assert!(!held(&rows).contains(&TableId::RailwayGunCounterInputs), "a frame that did not ask retires its tables");
}

#[test]
fn the_clock_speed_and_the_pickers_feature_read_rows_at_the_head() {
    let (entry, rows, watch) = opened("map");
    let head = Some((&entry, RECORDING));
    let today = nothing(1);
    let mut head_rows = HeadRows::default();
    let read = |head_rows: &mut HeadRows, head| (head_rows.map(head, Some(&today), MovementFrame::clock_speed), head_rows.map(head, Some(&today), crate::app::picker::four_buttons_at));
    assert_eq!(read(&mut head_rows, None), (Some(None), Some(true)), "off: today's frame records neither");
    assert_eq!(read(&mut head_rows, head), (None, None), "nothing, and never today's, while the rows load");
    let mut at_head = None;
    for hour in 2..38 {
        head_rows.end_frame();
        step(&rows, &watch, hour);
        if let (Some(Some(speed)), Some(four)) = read(&mut head_rows, head) {
            at_head = Some((speed, four));
            break;
        }
    }
    assert_eq!(at_head, Some((SPEED, false)), "the recorded speed, and features without the fourth button's");
}

#[test]
fn the_overlays_inputs_learn_their_tables_on_a_worker_and_a_mode_left_holds_nothing() {
    let (entry, rows, watch) = opened("overlay");
    let head = Some((&entry, RECORDING));
    let today = nothing(1);
    let mut head_rows = HeadRows::default();
    let resistance = OverlayInputs { priorities: PriorityTable::Resistance, resources: false, occupation: false, air: false, tables: false };
    let mut learned = false;
    let mut speed = None;
    for hour in 2..38 {
        let overlay = OverlayRows::at_head(head_rows.overlay(Some(resistance), head));
        let read = std::thread::scope(|scope| scope.spawn(|| overlay.read("clock", &today, MovementFrame::clock_speed)).join().unwrap());
        learned |= overlay.missed().contains(&TableId::Clock);
        head_rows.learn(&overlay.missed());
        head_rows.end_frame();
        if read.is_some() {
            speed = read;
            break;
        }
        step(&rows, &watch, hour);
    }
    assert!(learned, "the worker's read named the table its reader had not declared");
    assert_eq!(speed, Some(SPEED), "then read at the head");
    assert!(held(&rows).contains(&TableId::Clock));
    assert_eq!(head_rows.overlay(None, head), None, "no input reads rows");
    head_rows.end_frame();
    step(&rows, &watch, 39);
    assert!(!held(&rows).contains(&TableId::Clock), "the mode left retires its tables");
    assert_eq!(OverlayRows::default().read("clock", &today, MovementFrame::clock_speed), None, "off: today's frame");
}

#[test]
fn the_open_maps_overlay_tables_are_made_from_rows_at_the_head_alone_and_a_table_they_lack_keeps_what_was_made_and_is_named() {
    let (entry, rows, watch) = opened("overlay tables");
    let head = Some((&entry, RECORDING));
    let mut head_rows = HeadRows::default();
    let whole = OverlayInputs { priorities: PriorityTable::Resistance, resources: false, occupation: false, air: false, tables: true };
    assert_eq!(whole.without(true, true), Some(whole), "the open map's overlay is always read");
    let mut job: Job<usize, Option<i32>> = Job::default();
    let mut held: Option<Arc<DecodedFrame>> = None;
    let (mut named, mut made) = (false, None);
    for hour in 2..38 {
        if let Some(newest) = head_rows.overlay(Some(whole), head) {
            held = Some(newest);
        }
        if let Some(frame) = &held {
            let frame = Arc::clone(frame);
            let here = |work: Box<dyn FnOnce() + Send>| {
                work();
                true
            };
            job.ask(&(Arc::as_ptr(&frame) as usize), move || MovementFrame::clock_speed(&frame), &here, || {});
            match job.take() {
                Some((_, Outcome::Missed(missed))) => {
                    assert!(made.is_none(), "nothing made yet");
                    named |= job.loading().contains(&TableId::Clock);
                    head_rows.learn(&missed);
                }
                Some((_, Outcome::Made(speed))) => {
                    made = Some(speed);
                    break;
                }
                None => {}
            }
        }
        head_rows.end_frame();
        step(&rows, &watch, hour);
    }
    assert!(named, "the table the rows did not hold was named loading, and nothing was made from them");
    assert_eq!(made, Some(Some(SPEED)), "then made from rows at the head that hold it, with no frame of today's");
    assert!(job.loading().is_empty(), "nothing loading once made");
}

#[test]
fn a_saved_frames_overlay_at_a_later_stepped_hour_gets_rows_at_that_hour_holding_a_table_it_learned_there() {
    let (entry, rows, watch) = opened("settle at a later step");
    crate::app::playback_cost::hold_at_step(&entry, 1, 1.0);
    let head = Some((&entry, RECORDING));
    let mut head_rows = HeadRows::default();
    let whole = OverlayInputs { priorities: PriorityTable::Resistance, resources: false, occupation: false, air: false, tables: true };
    step(&rows, &watch, 2);
    let (first, ()) = head_rows.overlay_settled(whole, head, 2, SETTLE, |_| ((), Vec::new())).unwrap();
    assert_eq!(first.hour, 2, "the first stepped hour's rows");
    head_rows.end_frame();
    step(&rows, &watch, 3);
    let speed = |frame: &Arc<DecodedFrame>| replay_frame::frame::discovering(|| MovementFrame::clock_speed(frame));
    let (at, read) = head_rows.overlay_settled(whole, head, 3, SETTLE, speed).unwrap();
    assert_eq!((at.hour, read), (3, Some(SPEED)), "the table first read at the second hour joins the rows at that hour, not at a step past it");
}

/// The open map's overlay reader opens with every table the overlay is known to read, so at a held
/// head a build that reads them one after another is made in its first round. On the base the
/// reader declared nothing and learned one table a round: three rounds here.
#[test]
fn the_open_maps_overlay_is_made_in_one_round_at_a_held_head_from_the_tables_it_declares_up_front() {
    let (entry, _rows, _watch) = opened("overlay up front");
    let head = Some((&entry, RECORDING));
    let mut head_rows = HeadRows::default();
    let whole = OverlayInputs { priorities: PriorityTable::Resistance, resources: false, occupation: false, air: false, tables: true };
    let mut rounds = 0;
    let make = |frame: &Arc<DecodedFrame>| {
        rounds += 1;
        replay_frame::frame::discovering(|| {
            let divisions = frame.complete::<tables::Divisions>().ok()?.len();
            let features = frame.complete::<tables::SimulationFeatures>().ok()?.len();
            Some((divisions, features))
        })
    };
    let (rows, made) = head_rows.overlay_settled(whole, head, 1, SETTLE, make).unwrap();
    assert_eq!((rows.hour, made), (1, Some((1, 1))), "the held head's rows, holding both tables");
    assert_eq!(rounds, 1, "one build, no table learned");
}

/// Tables the default view's overlay build read beyond its declared ones in every paced run of
/// war24 (p25-overlay, REPORT2 section 1): learned a quarter second a round before.
const LEARNED_IN_RUNS: [TableId; 19] = [
    TableId::AirSortieBaseCountries,
    TableId::AirSortieModifierMetadata,
    TableId::AirSortieModifiers,
    TableId::AirSortieSpecialBases,
    TableId::ArmyOrderPlans,
    TableId::ArmyOrders,
    TableId::Characters,
    TableId::CoreSupplyAccess,
    TableId::CountryUnitSpeedEntries,
    TableId::GlobalSavedEventTargets,
    TableId::LeaderPortraitPaths,
    TableId::MovementStateTerms,
    TableId::NavalRepairBases,
    TableId::NavalRepairQueue,
    TableId::OrdersGroupCounterInputs,
    TableId::RailEdges,
    TableId::RiverRelations,
    TableId::ScriptVariables,
    TableId::TaskForceMapMotion,
];

/// The map's overlay reader opens when the map is opened, before its first frame is drawn: its
/// tables, those the runs found the default view's build reads among them, are declared to the
/// playhead with the map's own, the seek brings them, and its first rows are there before any
/// frame asked for them; frames drawn meanwhile keep it open. Opened again at a seek, the same
/// holds at the hour sought, and the overlay's first ask has those rows. On the base the reader
/// opened at the overlay's first ask, in the map's first drawn frame, without those tables.
#[test]
fn a_map_opened_or_sought_has_its_overlay_reader_open_and_its_first_rows_asked_for_before_its_first_frame() {
    let entry = open(record("overlay opened", 40), hourly()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let head = Some((&entry, RECORDING));
    let mut head_rows = HeadRows::default();
    let whole = OverlayInputs { priorities: PriorityTable::Resistance, resources: false, occupation: false, air: false, tables: true };
    head_rows.open_overlay(whole, head);
    assert!(rows.declared(&LEARNED_IN_RUNS, Instant::now() + LIMIT), "the overlay's reader declared every table the runs learned, before the map's first frame");
    for _ in 0..3 {
        head_rows.end_frame();
    }
    rows.drive(1);
    entry.playhead().seek(1);
    assert_eq!(head_rows.overlays[0].1.wait(|frame| frame.hour == 1, LIMIT).unwrap().hour, 1, "its first rows came with the seek, no frame having asked");
    head_rows.open_overlay(whole, head);
    head_rows.end_frame();
    rows.drive(20);
    entry.playhead().seek(20);
    head_rows.end_frame();
    head_rows.overlays[0].1.wait(|frame| frame.hour == 20, LIMIT).unwrap();
    assert_eq!(head_rows.overlay(Some(whole), head).map(|frame| frame.hour), Some(20), "the overlay's first ask after the seek has the rows of the hour sought");
}

/// Once the overlay's owner says when it has finished with its rows, the overlay's reader is made
/// none while it has not, however many steps change them, and one when it says so. Before the
/// first such call the reader is ready each frame the overlay looks. On the base it was ready each
/// frame whatever its tables job was doing, and made rows that job never read.
#[test]
fn the_overlays_reader_is_made_rows_only_when_its_owner_says_it_has_finished_with_them() {
    let entry = open(record("overlay ready", 40), hourly()).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, RECORDING).unwrap();
    let watch = rows.reader(&[TableId::Divisions], Cadence::EveryStep);
    let head = Some((&entry, RECORDING));
    let mut head_rows = HeadRows::default();
    let whole = OverlayInputs { priorities: PriorityTable::Resistance, resources: false, occupation: false, air: false, tables: true };
    head_rows.open_overlay(whole, head);
    rows.settled();
    rows.drive(1);
    entry.playhead().seek(1);
    watch.wait(|frame| frame.hour == 1, LIMIT).unwrap();
    head_rows.overlays[0].1.wait(|frame| frame.hour == 1, LIMIT).expect("its first rows");
    let taken = head_rows.overlay(Some(whole), head).expect("rows");
    head_rows.overlay_ready();
    let mut seen = vec![taken];
    for hour in 2..6 {
        step(&rows, &watch, hour);
        let newest = head_rows.overlay(Some(whole), head).expect("rows");
        head_rows.end_frame();
        if !seen.iter().any(|frame| Arc::ptr_eq(frame, &newest)) {
            seen.push(newest);
        }
    }
    assert!(seen.len() <= 2, "one frame for the one time it was ready, none after: {:?}", seen.iter().map(|frame| frame.hour).collect::<Vec<_>>());
    head_rows.overlay_ready();
    step(&rows, &watch, 6);
    let after = head_rows.overlays[0].1.wait(|frame| frame.hour == 6, LIMIT).expect("one frame once ready");
    assert_eq!(after.hour, 6);
}

/// A build of the overlay's tables in flight when other content opens gives that content nothing:
/// its tables were made from the recording before.
#[test]
fn an_overlay_build_in_flight_for_the_content_before_installs_nothing_after() {
    use crate::{
        Config, Options,
        app::{TablesKey, Viewer},
    };
    let mut viewer = Viewer::create(Config::default(), Options::default(), None, Arc::new(std::sync::Mutex::new(None)), None, None);
    let key = TablesKey {
        branch: 1,
        hour: 7,
        rows: 0,
        perspective: Default::default(),
        vision: 0,
        engaged: false,
        left_out: Default::default(),
        labels: false,
        need: viewer_counters::map::counters::Need { air_missions: false, by_key: true },
    };
    let held: std::sync::Mutex<Vec<Box<dyn FnOnce() + Send>>> = std::sync::Mutex::default();
    let submit = |work: Box<dyn FnOnce() + Send>| {
        held.lock().unwrap().push(work);
        true
    };
    viewer.overlay_tables_job.ask(&key, || (Err(io::Error::other("recording A")), None), &submit, || {});
    assert!(viewer.overlay_tables_job.running());
    viewer.forget_overlay_builds();
    let work = held.lock().unwrap().pop().expect("the build was handed to a worker");
    std::thread::spawn(work).join().unwrap();
    assert!(viewer.overlay_tables_job.take().is_none(), "recording A's build is not taken by the content after");
    viewer.overlay_tables_job.ask(&key, || (Err(io::Error::other("recording B")), None), &submit, || {});
    assert!(viewer.overlay_tables_job.running(), "the same key is built again for the content after");
}
