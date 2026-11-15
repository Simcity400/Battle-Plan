use super::{Ask, Scripts, Sources};
use crate::playhead_host::{
    Entry, open,
    rows::{Cadence, Recording, RowReader, Rows},
    units::Units,
};
use dashboard::triggers::World;
use installed_content_sound::music::{Music, Song, Track};
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
    rows::{Field, List, Opaque, RowCodec, RowsVisit, TableId, TableSchema, WarRelationRow, tables},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{collections::BTreeMap, io, sync::Arc, time::Duration};
use viewer_foundation::{map::perspective, trigger_content::TriggerContent};
use viewer_raids::map::raids::{self, test_support};

const RECORDING: Recording = Recording { branch: 3 };
const LIMIT: Duration = Duration::from_secs(60);
/// Hours recorded; the head moves an hour at a time until the scripts have learned their tables.
const HOURS: u32 = 120;

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

/// Every string a row of `frame` holds.
struct Strings<'a>(&'a mut BTreeMap<Vec<u8>, u32>);

impl RowsVisit for Strings<'_> {
    type Output = ();
    fn rows<R: RowCodec>(self, rows: &[R]) {
        for row in rows {
            row.visit_wire(&mut |_, field| {
                if let Field::Bytes(bytes) = field {
                    let next = self.0.len() as u32;
                    self.0.entry(bytes.to_vec()).or_insert(next);
                }
                Ok(())
            })
            .unwrap();
        }
    }
}

/// A table's rows written into the store, each field as the recorder writes it.
struct Put<'a> {
    store: &'a mut Store,
    table: TableId,
    strings: &'a BTreeMap<Vec<u8>, u32>,
}

impl RowsVisit for Put<'_> {
    type Output = ();
    fn rows<R: RowCodec>(self, rows: &[R]) {
        let mut changes = self.store.table(self.table.index()).unwrap();
        let mut opaque = Vec::new();
        for (slot, row) in rows.iter().enumerate() {
            let slot = slot as u32;
            changes.insert_at(slot, row.key()).unwrap();
            row.visit_wire(&mut |column, field| {
                match field {
                    Field::NotRecorded => changes.put_value(slot, column, Value::Unrecorded),
                    Field::Empty => changes.put_value(slot, column, Value::Missing),
                    Field::Opaque(Opaque::Encoded(bytes, _)) => changes.write(slot, column, Cell::Opaque(bytes)),
                    Field::Opaque(Opaque::Native(value)) => {
                        opaque.clear();
                        value.encode_value(&mut opaque)?;
                        changes.write(slot, column, Cell::Opaque(&opaque))
                    }
                    field => changes.write(slot, column, cell(field, self.strings)),
                }
            })
            .unwrap();
        }
    }
}

fn cell<'a>(field: Field<'a>, strings: &BTreeMap<Vec<u8>, u32>) -> Cell<'a> {
    match field {
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
        Field::Bytes(bytes) => Cell::Bytes(strings[bytes]),
        Field::U32List(List::Native(items)) => Cell::U32List(items),
        Field::U64List(List::Native(items)) => Cell::U64List(items),
        _ => panic!("the fixture holds a field the test cannot write"),
    }
}

/// `frame`'s tables recorded for `HOURS` hours from hour 0 as the recorder writes them: each
/// complete table's rows at hour 0 and its status every hour, a table not complete with its status.
fn record(name: &str, frame: &DecodedFrame) -> Units {
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut strings = BTreeMap::new();
    for view in &frame.tables {
        match &**view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.visit(Strings(&mut strings)),
            TableView::Unavailable { reason } => {
                let next = strings.len() as u32;
                strings.entry(reason.as_bytes().to_vec()).or_insert(next);
            }
            TableView::NotRecorded { .. } => {}
        }
    }
    let mut named: Vec<(&Vec<u8>, &u32)> = strings.iter().collect();
    named.sort_by_key(|(_, id)| **id);
    let named: Vec<&[u8]> = named.into_iter().map(|(bytes, _)| bytes.as_slice()).collect();
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing::at(3)).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..HOURS {
        store.begin_hour(at as i32, if at == 0 { &named } else { &[] }).unwrap();
        for table in TableId::ALL {
            let view = &*frame.tables[table.index()];
            let status = match view {
                TableView::Complete(_) => Status::Complete,
                TableView::Truncated { missing, .. } => Status::Truncated { missing: *missing },
                TableView::Unavailable { reason } => Status::Unavailable { reason: strings[reason.as_bytes()] },
                TableView::NotRecorded { .. } => Status::NotRecorded,
            };
            store.status(table.index(), status, true, Some(at as i32), true).unwrap();
            if at == 0
                && let TableView::Complete(rows) | TableView::Truncated { rows, .. } = view
            {
                rows.visit(Put { store: &mut store, table, strings: &strings });
            }
        }
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    if !HOURS.is_multiple_of(BLOCK) {
        store.end_run(&mut memory).unwrap();
    }
    let run = Run::new(0, memory.pages).unwrap();
    let kinds = Arc::new(kinds);
    let pages: Arc<dyn Pages> = Arc::new(Recorded { bytes: memory.bytes, runs: vec![run], kinds: (*kinds).clone() });
    let timeline = Timeline::whole(&*pages).unwrap();
    Units { name: format!("scripts test {name}"), pages, timeline, kinds }
}

/// An hour a step.
fn hourly_steps() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0 }
}

/// Two songs: a base chance, and one doubled while the viewing country is at war with ENG.
fn music() -> Arc<viewer_audio::Content> {
    let mut music = Music::default();
    for (index, body) in ["base=1", "base=7 modifier={has_war_with=ENG factor=2}"].into_iter().enumerate() {
        music.tracks.push(Track { name: format!("t{index}"), path: format!("music/t{index}.ogg"), volume: 1.0, source: String::new() });
        music.songs.push(Song { track: index, station: "on".into(), chance: installed_content_core::parser::parse(body.as_bytes()).unwrap().items, source: String::new() });
    }
    Arc::new(viewer_audio::Content::new(Default::default(), None).with_music(music, Some(vec!["on".into()]), 0))
}

/// The raid fixture with GER (raw 10) at war with ENG (raw 11), on the test recording's branch.
fn fixture() -> DecodedFrame {
    let mut frame = test_support::frame();
    test_support::put::<tables::WarRelations>(&mut frame, vec![WarRelationRow { first_country: 10, second_country: 11, start_hour: 0, end_hour: i32::MAX, ..Default::default() }]);
    frame.branch = RECORDING.branch;
    frame
}

struct Fixture {
    entry: Arc<Entry>,
    rows: Arc<Rows>,
    /// A reader every step, which the test waits on to know the head reached an hour.
    watch: RowReader,
    raids: Arc<raids::RaidAssets>,
    triggers: Arc<TriggerContent>,
    audio: Arc<viewer_audio::Content>,
    tables: engine_executable::executable_tables::Tables,
    hour: Hour,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let raids = Arc::new(test_support::assets());
        let tables = engine_executable::executable_tables::initialize_for_tests().unwrap();
        let triggers = Arc::new(raids.trigger_content.clone());
        let entry = open(record(name, &fixture()), hourly_steps()).unwrap();
        entry.set_capture(CaptureMode::Full);
        let rows = Rows::of(&entry, RECORDING).unwrap();
        let watch = rows.reader(&[TableId::Countries], Cadence::EveryStep);
        rows.settled();
        rows.drive(1);
        entry.playhead().seek(1);
        watch.wait(|frame| frame.hour == 1, LIMIT).unwrap();
        Self { entry, rows, watch, raids, triggers, audio: music(), tables, hour: 1 }
    }

    fn ask(&self, playing: bool) -> Ask<'_> {
        static CHOICE: std::sync::LazyLock<perspective::Choice> = std::sync::LazyLock::new(perspective::Choice::default);
        Ask { entry: &self.entry, recording: RECORDING, sources: Sources { raids: &self.raids, triggers: &self.triggers, audio: Some(&self.audio), tables: Some(self.tables) }, perspective: &CHOICE, playing, raids: true, music: true }
    }

    /// The head an hour on, reached.
    fn step(&mut self) {
        self.hour += 1;
        self.rows.drive(self.hour);
        self.watch.wait(|frame| frame.hour == self.hour, LIMIT).unwrap();
    }

    /// Whether every table the scripts declare has joined: the rows' worker holds its mirror. The
    /// scripts' worker declares off this thread, so this waits for the declaration first: a head
    /// moved before it would spend the recording's hours with nothing to join.
    fn joined(&self, tables: &[TableId], deadline: std::time::Instant) -> bool {
        assert!(self.rows.declared(tables, deadline), "the scripts' reader did not declare its tables to the playhead within {LIMIT:?}: {tables:?}");
        let held = self.rows.held();
        tables.iter().all(|table| held.mirrors.iter().any(|(mirrored, _)| mirrored == table))
    }
}

/// Today's evaluation over today's frame at `hour`: the raid icons with every target evaluated,
/// and the music chances of `ours`.
fn today(fixture: &Fixture, hour: Hour, ours: u32) -> (String, String) {
    engine_executable::executable_tables::scope(Some(fixture.tables), || today_in(fixture, hour, ours))
}

/// The tables today's evaluation reads of the fixture, found round by round as the worker finds
/// them: over a frame whose every table not found yet is undeclared.
fn read_by(fixture: &Fixture, ours: u32) -> Vec<TableId> {
    let mut found: Vec<TableId> = Vec::new();
    loop {
        let mut frame = self::fixture();
        for table in TableId::ALL.into_iter().filter(|table| !found.contains(table)) {
            frame.tables[table.index()] = replay_frame::frame::not_declared(table);
        }
        let (_, missed) = engine_executable::executable_tables::scope(Some(fixture.tables), || {
            replay_frame::frame::discovering(|| {
                let _ = super::ours(&frame, &perspective::Choice::default());
                let paths = raids::paths::HourPaths::new(&fixture.raids, &frame);
                let _ = raids::icons(&fixture.raids, &frame, ours, &paths);
                let world = World::read(&frame, fixture.triggers.facts());
                let _ = map_products::music_inputs::Memo::new().chances(&frame, &fixture.audio, ours, &world);
            })
        });
        if missed.is_empty() {
            return found;
        }
        found.extend(missed);
    }
}

fn today_in(fixture: &Fixture, hour: Hour, ours: u32) -> (String, String) {
    let mut frame = self::fixture();
    frame.hour = hour;
    let paths = raids::paths::HourPaths::new(&fixture.raids, &frame);
    let icons = raids::icons(&fixture.raids, &frame, ours, &paths);
    let world = World::read(&frame, fixture.triggers.facts());
    let music = map_products::music_inputs::Memo::new().chances(&frame, &fixture.audio, ours, &world);
    (format!("{:?} {:?}", icons.icons, icons.report), format!("{:?} {:?}", music.weights, music.refusals))
}

#[test]
fn the_script_readers_read_rows_at_the_head_learning_their_tables_and_close_with_the_view() {
    let mut fixture = Fixture::new("readers");
    let ours = super::ours(&self::fixture(), &perspective::Choice::default()).unwrap();
    // Primed with every table today's evaluation reads but one, which the readers learn at the
    // head (sixty tables joining at once take the playhead a minute in a test build).
    let read = read_by(&fixture, ours);
    assert!(read.contains(&tables::WarRelations::ID), "{read:?}");
    let primed: Vec<TableId> = read.iter().copied().filter(|&table| table != tables::WarRelations::ID).collect();
    let mut scripts = Scripts::primed(&primed.iter().map(|&table| crate::playhead_host::rows::Need::keys(table)).collect::<Vec<_>>());
    scripts.show(fixture.ask(false), || {});
    assert!(scripts.raids(RECORDING.branch, ours).is_none() && scripts.music().is_none(), "nothing while the tables load: the window shows today's");
    // The head moves on until every table the scripts read has joined and they evaluated it. A
    // table learned while playing joins only after the steps already built ahead of the head (at
    // most the ring), so the head moves whenever one has not joined; the recording leaves room.
    let ahead = hourly_steps().ring as Hour;
    let deadline = std::time::Instant::now() + LIMIT;
    let (icons, music) = loop {
        let tables = scripts.tables();
        assert!(std::time::Instant::now() < deadline, "the scripts did not evaluate at the head within {LIMIT:?}: hour {}, declaring {tables:?}", fixture.hour);
        if !fixture.joined(&tables, deadline) {
            assert!(fixture.hour + ahead < HOURS as Hour, "the recording ran out of hours before the scripts' tables joined (the ring builds {ahead} steps ahead): {tables:?}");
            fixture.step();
            continue;
        }
        let rounds = scripts.rounds();
        scripts.show(fixture.ask(false), || {});
        scripts.end_frame();
        let at = |scripts: &Scripts| {
            let icons = scripts.raids(RECORDING.branch, ours).filter(|icons| icons.built_for.is_some_and(|(_, hour, _)| hour == fixture.hour) && icons.report.cycle_complete());
            let music = scripts.music_at().filter(|(hour, _)| *hour == fixture.hour);
            match (icons, music, scripts.tables() == tables) {
                (Some(icons), Some((_, music)), true) => Some((format!("{:?} {:?}", icons.icons, icons.report), format!("{:?} {:?}", music.weights, music.refusals))),
                _ => None,
            }
        };
        // Evaluated already, or evaluated in the round this frame lets run; a table learned
        // joins only when the head moves on.
        let reached = at(&scripts).or_else(|| {
            if scripts.tables() != tables {
                return None;
            }
            scripts.wait_round(rounds, deadline.saturating_duration_since(std::time::Instant::now()));
            at(&scripts)
        });
        if let Some(reached) = reached {
            break reached;
        }
    };
    let learned = scripts.tables();
    assert!(learned.contains(&tables::WarRelations::ID) && learned.contains(&tables::RaidTargetStatuses::ID), "learned by reading: {learned:?}");
    assert_eq!((icons, music), today(&fixture, fixture.hour, ours), "the same as today's evaluation over today's frame of the hour");
    let held = fixture.rows.held();
    assert!(held.mirrors.iter().any(|(table, _)| *table == tables::WarRelations::ID));
    // A frame that does not ask closes the readers off this thread: wait for the reader to be gone.
    scripts.end_frame();
    assert!(fixture.rows.released(tables::WarRelations::ID, std::time::Instant::now() + LIMIT), "the scripts' reader closes with the frame that did not ask");
    assert!(scripts.raids(RECORDING.branch, ours).is_none() && scripts.music().is_none(), "closed, it holds nothing");
    fixture.step();
    let held = fixture.rows.held();
    assert_eq!(held.mirrors.iter().map(|(table, _)| *table).collect::<Vec<_>>(), vec![TableId::Countries], "only the watch's table is held");
}
