//! The mode fills layer on a recording made in memory, with a painter standing in for today's
//! political part: at the default mode it declares and holds nothing; at another it declares what
//! the painter reads, shows nothing until its first picture and then that picture's hour; a mode
//! change builds for the new mode and never shows the old one's picture; a table the recording
//! holds nothing of refuses it by name.

use super::{Fill, ModeFills, Paint, TABLES};
use crate::{
    layers::{Layer, Published, View, recorded},
    playhead_host::{MapSide, open, rows::Recording, units::Units},
};
use engine_executable::executable_tables::Tables;
use map_display_data::{
    landscape::Regions,
    province_field::{ProvinceField, Storage},
};
use playhead::{Settings, Timeline};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::Store,
};
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use std::{
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use viewer_foundation::map::perspective::Choice;
use viewer_world::map::map_mode::{MapMode, Stock};

const LIMIT: Duration = Duration::from_secs(120);

/// Shut, a picture waits at it, so the worker holds its build; shut past the limit, the build fails
/// by name.
#[derive(Default)]
struct Gate {
    /// Whether it is open, and whether a build has reached it.
    state: Mutex<(bool, bool)>,
    moved: Condvar,
}

impl Gate {
    fn open(&self) {
        self.state.lock().unwrap().0 = true;
        self.moved.notify_all();
    }

    fn reached(&self) -> bool {
        self.state.lock().unwrap().1
    }

    fn pass(&self) -> io::Result<()> {
        let until = Instant::now() + LIMIT;
        let mut state = self.state.lock().unwrap();
        state.1 = true;
        while !state.0 {
            let now = Instant::now();
            if now >= until {
                return Err(io::Error::other("the test never opened the gate"));
            }
            state = self.moved.wait_timeout(state, until - now).unwrap().0;
        }
        Ok(())
    }
}

/// Opens the gate when the test ends, so the worker's build ends and the layer can stop.
struct Opens(Arc<Gate>);

impl Drop for Opens {
    fn drop(&mut self) {
        self.0.open();
    }
}

/// Passes the gate, reads `reads` and paints an empty picture of the mode asked.
struct Painter {
    reads: Vec<TableId>,
    gate: Arc<Gate>,
}

impl Paint for Painter {
    fn tables(&self) -> Option<Tables> {
        None
    }

    fn paint(&self, frame: &DecodedFrame, _choice: &Choice, mode: &MapMode, _selected: Option<u32>) -> io::Result<Fill> {
        self.gate.pass()?;
        for table in &self.reads {
            let _ = std::hint::black_box(&*frame.tables[table.index()]);
        }
        let field = || ProvinceField(Arc::new(Storage::Dense(Arc::new(Vec::new()))));
        let regions = Regions { country_of: field(), owner_of: field(), state_of: field(), country_colors: Default::default(), warnings: Vec::new(), strategic_attributes: Default::default(), mode: None };
        Ok(Fill { mode: mode.clone(), regions: Arc::new(regions), zone: map_display_data::retained::Retained::new(Default::default()), classes: Default::default(), highlighted: Default::default() })
    }
}

/// Forty hours of the Clock and the territory's tables, held whole and empty; a recording of its
/// own name.
fn record() -> Units {
    static RECORDED: AtomicUsize = AtomicUsize::new(0);
    let name = format!("mode fills test {}", RECORDED.fetch_add(1, Ordering::Relaxed));
    let mut tables: Vec<TableId> = TABLES.to_vec();
    tables.push(TableId::Clock);
    let pages = crate::layers::rails::tests::recorded(40, |_| Vec::new(), |at, store: &mut Store| {
        if at == 0 {
            store.table(TableId::Clock.index()).unwrap().insert_at(0, 0).unwrap();
            store.table(TableId::Clock.index()).unwrap().write(0, recorded::column(TableId::Clock, "speed"), Cell::I32(1)).unwrap();
        }
        for &table in &tables {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
    });
    let kinds = Arc::new((0..TableId::ALL.len()).map(|table| pages.kinds(table).map(<[Kind]>::to_vec).unwrap_or_default()).collect());
    let timeline = Timeline::whole(&*pages).unwrap();
    Units { name, pages, timeline, kinds }
}

fn view(mode: Stock) -> View {
    View { map_mode: MapMode::Stock(mode), ..View::default() }
}

/// The layer as the window makes it, alone on the map, at `at`, given its first reset.
fn side_of(units: Units, painter: Painter, at: View) -> MapSide {
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0 }).unwrap();
    let layer = super::new(Arc::new(painter), false, false).unwrap().in_window(Arc::clone(&entry), Recording { branch: 1 }).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(layer)], at).unwrap();
    assert!(side.refusals().next().is_none());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    until(&mut side, "the first reset swapped in", |side, frame| frame.swapped.is_some() && !side.composing());
    side
}

/// Frames at the newest step built until `done` holds; fails naming `what` after 120 s.
fn until(side: &mut MapSide, what: &str, mut done: impl FnMut(&MapSide, &crate::playhead_host::Frame) -> bool) {
    let deadline = Instant::now() + LIMIT;
    loop {
        let head = side.entry().playhead().newest().unwrap_or(0);
        let frame = side.frame(head).unwrap();
        if done(side, &frame) {
            return;
        }
        assert!(Instant::now() < deadline, "not reached within 120 s: {what}");
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
}

fn fills(side: &MapSide) -> &ModeFills {
    side.parts().unwrap().1[0].any().downcast_ref::<ModeFills>().unwrap()
}

fn shown_mode(side: &MapSide) -> Option<MapMode> {
    fills(side).product().and_then(|built| built.product.as_ref().ok()).map(|fill| fill.mode.clone())
}

#[test]
fn a_mode_fill_shows_its_own_modes_picture_from_its_first_product_and_nothing_at_the_default() {
    let gate = Arc::new(Gate::default());
    let _opens = Opens(Arc::clone(&gate));
    let air = view(Stock::StrategicAir);
    let mut side = side_of(record(), Painter { reads: vec![TableId::Subjects], gate: Arc::clone(&gate) }, air.clone());

    // Another mode: it reads the Clock at the head and builds on its worker, which the gate holds.
    assert!(side.parts().unwrap().0.get(TableId::Clock).is_some(), "at another mode it declares the Clock");
    until(&mut side, "the worker's first build", |_, _| gate.reached());
    assert_eq!(fills(&side).published(), Published::Nothing, "before its first picture it has published nothing");
    assert!(fills(&side).product().is_none());
    assert_eq!(fills(&side).carried().entities, 0, "the worker's build is not the frame thread's work");
    assert_eq!(fills(&side).lines(), vec!["map mode fill: loading, no picture of map mode strategic_air at the head yet".to_owned()], "loading, by name");

    gate.open();
    let hour = side.applied().expect("the head has reached an hour");
    side.frame(hour).unwrap();
    let built = fills(&side).rebuilt().wait(hour, LIMIT).unwrap();
    assert_eq!(built.product.as_ref().map(|fill| fill.mode.clone()).ok(), Some(air.map_mode.clone()), "built for the view's mode: {:?}", built.product.as_ref().err());
    side.settle_all().unwrap();
    assert_eq!(fills(&side).published(), Published::Cadence(hour), "once built it shows the hour of its rows");
    assert_eq!(shown_mode(&side), Some(air.map_mode.clone()));
    let tables = fills(&side).rebuilt().tables();
    assert!(TABLES.iter().all(|table| tables.contains(table)), "it declares the tables the painter reads: {tables:?}");
    assert_eq!(fills(&side).refusal(), None);

    // A mode change takes the old mode's picture down at once and shows only the new mode's.
    let resources = view(Stock::Resources);
    side.set_view(resources.clone()).unwrap();
    assert!(fills(&side).product().is_none(), "the old mode's picture is let go with the change");
    assert_eq!(fills(&side).published(), Published::Nothing);
    let deadline = Instant::now() + LIMIT;
    while shown_mode(&side).is_none() {
        assert!(Instant::now() < deadline, "no picture of the new mode within 120 s");
        side.frame(hour).unwrap();
        std::thread::yield_now();
    }
    assert_eq!(shown_mode(&side), Some(resources.map_mode.clone()), "the first picture after the change is the new mode's, never the old one's");

    // The default mode is the political layer's: it declares, holds and carries nothing there.
    side.set_view(View::default()).unwrap();
    side.frame(hour).unwrap();
    assert!(side.parts().unwrap().0.get(TableId::Clock).is_none(), "at the default mode it declares no table");
    assert!(fills(&side).product().is_none(), "nor holds a picture");
    assert_eq!(fills(&side).held(), 0);
    assert_eq!(fills(&side).carried().entities, 0);
    assert_eq!(fills(&side).published(), Published::Current, "drawing nothing, it holds no hour back");
    assert!(fills(&side).lines().is_empty());

    // A table the recording holds nothing of refuses the layer by name. A table the store never
    // wrote is still in every block's page, whole and empty: it is taken out of them.
    let gate = Arc::new(Gate::default());
    gate.open();
    let units = crate::layers::rebuilt::tests::without(record(), TableId::Factions);
    let mut side = side_of(units, Painter { reads: vec![TableId::Factions], gate }, air);
    until(&mut side, "the refused picture taken", |side, _| fills(side).product().is_some());
    let why = fills(&side).refusal().map(str::to_owned).unwrap_or_else(|| {
        let taken = fills(&side).product().map(|built| (built.hour, built.product.as_ref().map(|fill| fill.mode.token()).map_err(Clone::clone)));
        panic!(
            "the picture is not refused: the layer took {taken:?} (hour, the mode drawn or the refusal), its lines {:?}, its tables {:?}",
            fills(&side).lines(),
            fills(&side).rebuilt().tables()
        )
    });
    assert!(why.contains("holds no Factions") && why.contains("mode_fills"), "the refusal names the table and the layer: {why}");
    assert!(fills(&side).lines().iter().any(|line| line.contains("holds no Factions")), "{:?}", fills(&side).lines());
    assert!(!fills(&side).rebuilt().tables().contains(&TableId::Factions), "the table is never declared");
}
