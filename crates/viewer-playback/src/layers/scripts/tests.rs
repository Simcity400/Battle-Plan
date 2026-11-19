use super::{Held, Scripts};
use crate::layers::{Layer, Settling};
use crate::playhead_host::{
    open,
    units::Units,
    views::{Need, Recording, Views, apart::Apart},
};
use dashboard_text::triggers::World;
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
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }
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
    /// A view store of this playhead, driven to the hours the test asks for, as the map side's
    /// is by its step.
    fed: Apart,
    views: Views,
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
        entry.playhead().seek(1);
        let mut fed = Apart::new(&entry, RECORDING.branch).unwrap();
        assert!(fed.drive(1), "the store never reached hour 1");
        let views = fed.store().handle();
        Self { fed, views, raids, triggers, audio: music(), tables, hour: 1 }
    }

    /// What the scripts evaluate with, as the layer is given it from the document's content.
    fn content(&self) -> Held {
        Held { raids: Arc::clone(&self.raids), triggers: Arc::clone(&self.triggers), audio: Some(Arc::clone(&self.audio)), tables: Some(self.tables) }
    }

    /// The layer opened on this store, drawn, at the hour the store stands at.
    fn scripts(&self, needs: &[Need]) -> Scripts {
        let mut scripts = Scripts::with(self.content(), needs.to_vec());
        scripts.views = Some(self.views.clone());
        scripts.shown = (true, true);
        scripts.applied = Some(self.hour);
        // The music chances' rate is read against a clock this test steps, so what it withholds
        // never depends on how long the test took.
        scripts.clock = super::Clock::Stepped(Duration::ZERO);
        scripts
    }

    /// The views its readers ask for, posted again at the hour it stands at: what a publication
    /// the head stands still for does.
    fn posted(&mut self) {
        assert!(self.fed.drive(self.hour), "the store never posted hour {}", self.hour);
    }

    /// The head an hour on, reached, and its views posted.
    fn step(&mut self) {
        self.hour += 1;
        assert!(self.fed.drive(self.hour), "the store never reached hour {}", self.hour);
    }

    /// The tables the store keeps views of now.
    fn held(&self) -> Vec<TableId> {
        self.fed.store().held().iter().map(|(table, _)| *table).collect()
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

/// The layer asked and joined as the map side's step asks and joins it: the newest rows offered to
/// the worker at the apply, then what it published taken, waiting for the hour applied when the
/// publication waits ([`Layer::ask`], [`Layer::join`]).
fn asked(scripts: &mut Scripts, settling: Settling) -> io::Result<()> {
    let mirrors = crate::playhead_host::mirror::Mirrors::default();
    let upstream = crate::layers::Upstream::new(&[]);
    assert!(Layer::ask(scripts, &mirrors, &upstream)?, "the scripts join a maker of their own");
    Layer::join(scripts, &mirrors, &upstream, settling)
}

/// A publication at the hour after the one before: the hour's steps applied and the views its
/// readers asked for posted, then the layer asked and joined.
fn step(fixture: &mut Fixture, scripts: &mut Scripts, settling: Settling) -> io::Result<()> {
    scripts.ensure_open();
    fixture.step();
    scripts.applied = Some(fixture.hour);
    asked(scripts, settling)
}

/// What the layer published, as the proof compares it: the icons and the music chances in words.
fn published(scripts: &Scripts, ours: u32, hour: Hour) -> Option<(String, String)> {
    let icons = scripts.raids(RECORDING.branch, ours).filter(|icons| icons.built_for.is_some_and(|(_, at, _)| at == hour) && icons.report.cycle_complete())?;
    let (_, music) = scripts.music_at().filter(|(at, _)| *at == hour)?;
    Some((format!("{:?} {:?}", icons.icons, icons.report), format!("{:?} {:?}", music.weights, music.refusals)))
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
    let mut scripts = fixture.scripts(&primed.iter().map(|&table| Need::keys(table)).collect::<Vec<Need>>());
    assert!(scripts.raids(RECORDING.branch, ours).is_none() && scripts.music_at().is_none(), "nothing while the tables load: the window shows today's");
    // The head moves on until the scripts have learned every table they read and a publication
    // the head stands still for has the icons of its own hour.
    let reached = loop {
        let tables: Vec<TableId> = scripts.needs().iter().map(|need| need.table).collect();
        assert!(fixture.hour + 2 < HOURS as Hour, "the recording ran out of hours before the scripts evaluated: {tables:?}");
        step(&mut fixture, &mut scripts, Settling::still()).unwrap();
        if let Some(reached) = published(&scripts, ours, fixture.hour) {
            break reached;
        }
    };
    let learned: Vec<TableId> = scripts.needs().iter().map(|need| need.table).collect();
    assert!(learned.contains(&tables::WarRelations::ID) && learned.contains(&tables::RaidTargetStatuses::ID), "learned by reading: {learned:?}");
    assert_eq!(reached, today(&fixture, fixture.hour, ours), "the same as today's evaluation over today's frame of the hour");
    assert!(fixture.held().contains(&tables::WarRelations::ID), "the store keeps the views the scripts read");
    // Neither raid icons nor music are drawn: the reader closes off this thread, and nothing is held.
    scripts.shown = (false, false);
    // Waited for by the hand-over itself: the thread the close handed back frees the reader.
    if let Some(closing) = scripts.close() {
        closing.join().expect("the scripts' closing thread");
    }
    assert!(!fixture.held().contains(&tables::WarRelations::ID), "closed, the scripts' reader lets its tables go");
    assert!(scripts.raids(RECORDING.branch, ours).is_none() && scripts.music_at().is_none(), "closed, it holds nothing");
    fixture.step();
    assert!(fixture.held().is_empty(), "no reader is left, so the store keeps no view: {:?}", fixture.held());
}

/// A stopped picture (a publication the head stands still for, a saved frame, the proof) shows the
/// scripts' product of its own hour, with every target of the offer cycle evaluated: the step
/// waits for it. While the hours run the rate holds the evaluations to its own count and the
/// product stands at an hour no older than the rate allows.
#[test]
fn a_stopped_picture_shows_the_scripts_product_of_its_own_hour() {
    let mut fixture = Fixture::new("stopped");
    let ours = super::ours(&self::fixture(), &perspective::Choice::default()).unwrap();
    let primed: Vec<Need> = read_by(&fixture, ours).iter().map(|&table| Need::every(table)).collect();
    let mut scripts = fixture.scripts(&primed);
    // Every publication the head stands still for shows the hour it stands at, from the first the
    // scripts' tables are all home for.
    let mut stopped = 0;
    while stopped < 3 {
        assert!(fixture.hour + 2 < HOURS as Hour, "the recording ran out of hours");
        step(&mut fixture, &mut scripts, Settling::still()).unwrap();
        let Some(reached) = published(&scripts, ours, fixture.hour) else { continue };
        assert_eq!(reached, today(&fixture, fixture.hour, ours), "hour {}: the stopped picture is today's evaluation of its own hour", fixture.hour);
        assert_eq!(crate::layers::Layer::published(&scripts), crate::layers::Published::Current, "its product stands at the rows it has seen");
        stopped += 1;
    }
    // Playing, the raid cycle is evaluated at every publication: its icons are on screen, so the
    // product keeps the hour. The music chances are the rated product and their rate withholds
    // most of the reads.
    let evaluations: u64 = scripts.stats().evaluations.values().sum();
    for _ in 0..8 {
        // As the side settles: a product the bound's publications behind is waited for.
        let behind = (fixture.hour + 1 - scripts.product().expect("published above").hour) as u32;
        step(&mut fixture, &mut scripts, Settling::step().behind_bound(behind, crate::layers::BOUND)).unwrap();
        let behind = fixture.hour - scripts.product().expect("published above").hour;
        assert!((0..crate::layers::BOUND as Hour).contains(&behind), "{behind} hours behind at hour {}", fixture.hour);
    }
    assert!(scripts.stats().evaluations.values().sum::<u64>() > evaluations, "the cycle is evaluated at the publication's rate");
    assert!(scripts.stats().rated > 0, "the music chances' rate withheld reads of them while the hours ran");
    // The rate's own clock decides that, and nothing else: inside its interval the chances are
    // withheld, and once the clock has passed it they are read again.
    let withheld = scripts.stats().rated;
    assert!(!scripts.music_due(false), "inside its interval the rate withholds the chances");
    assert_eq!(scripts.stats().rated, withheld + 1);
    scripts.clock.step(super::MUSIC_RATE.interval());
    assert!(scripts.music_due(false), "its interval elapsed, the chances are read again");
    assert_eq!(scripts.stats().rated, withheld + 1, "and nothing more is withheld");
    // The hours stop: the first publication that waits asks for the rows of the hour it shows and
    // the one after it shows the scripts' product of that hour, as the rows of an hour are made by
    // the step that asks for them. Nothing older is ever shown as the hour's.
    step(&mut fixture, &mut scripts, Settling::still()).unwrap();
    let stopped_at = fixture.hour;
    scripts.applied = Some(stopped_at);
    for _ in 0..2 {
        fixture.posted();
        asked(&mut scripts, Settling::still()).unwrap();
    }
    assert_eq!(published(&scripts, ours, stopped_at), Some(today(&fixture, stopped_at, ours)), "the picture the hours stopped on is its own hour's");
}
