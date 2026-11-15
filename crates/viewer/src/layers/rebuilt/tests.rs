//! A layer rebuilt whole on its worker, read after the vision layer on a recording made in memory:
//! while its worker holds a build it carries nothing and has published nothing, once built it shows
//! the hour of its rows, and a table its builder reads that the recording holds nothing of is
//! refused by name, never declared. Builds run under the recording's executable tables, a build
//! that panics is a named refusal of a worker that goes on, and one round learns every table a
//! build reaches. A worker builds nothing while no table it reads changes, its product then
//! standing at the newer hours, and with a step every frame it makes the product of each step's
//! rows, waiting for no clock.

use super::{Build, Seen, Whole};
use engine_executable::executable_tables::Tables;
use crate::{
    layers::{Layer, Published, View, recorded, vision},
    playhead_host::{MapSide, open, rows::Cadence, rows::Recording, units::Units},
};
use playhead::{
    Settings, Timeline,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::Store,
    unit::{TableEntry, reader::Extents},
};
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use viewer_foundation::map::perspective::{Choice, VISION_TABLES, Vision};

/// Shut, a build waits at it, so the worker holds its work; a minute shut fails the build by name.
#[derive(Default)]
struct Gate {
    /// Whether it is open, and whether a build has reached it.
    state: Mutex<(bool, bool)>,
    moved: Condvar,
}

impl Gate {
    fn shut() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn opened() -> Arc<Self> {
        let gate = Self::shut();
        gate.open();
        gate
    }

    fn open(&self) {
        self.state.lock().unwrap().0 = true;
        self.moved.notify_all();
    }

    fn reached(&self) -> bool {
        self.state.lock().unwrap().1
    }

    fn pass(&self) -> io::Result<()> {
        let until = Instant::now() + Duration::from_secs(60);
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

/// Passes the gate, reads `reads` and answers the hour of its rows.
struct Probe {
    reads: Vec<TableId>,
    gate: Arc<Gate>,
}

impl Build for Probe {
    type Product = Hour;

    fn tables(&self) -> Option<Tables> {
        None
    }

    fn build(&self, frame: &DecodedFrame, _seen: &Seen) -> io::Result<Hour> {
        self.gate.pass()?;
        for table in &self.reads {
            let _ = std::hint::black_box(&*frame.tables[table.index()]);
        }
        Ok(frame.hour)
    }
}

/// A recording's executable tables: every one withheld, as of an image that is no executable.
fn recording_tables() -> Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    let decoded = || engine_executable::executable_tables::decode(engine_executable::build::ExecutableIdentity::of(b"rebuilt test"), Vec::new(), std::path::Path::new("no game install"));
    *TABLES.get_or_init(|| -> Tables { Box::leak(Box::new(decoded())) })
}

/// Builds only under the recording's tables; outside them it stops as a build reaching the trigger
/// registries does.
struct NeedsTables {
    given: Option<Tables>,
}

impl Build for NeedsTables {
    type Product = Hour;

    fn tables(&self) -> Option<Tables> {
        self.given
    }

    fn build(&self, frame: &DecodedFrame, _seen: &Seen) -> io::Result<Hour> {
        let current = engine_executable::executable_tables::current();
        assert!(current.is_some_and(|current| std::ptr::eq(current, recording_tables())), "trigger and effect registries withheld: no recording's tables are in scope for this work");
        Ok(frame.hour)
    }
}

/// Reads `reads` in order and stops at the first not held, as today's plan stops at the first
/// table it misses.
struct InOrder {
    reads: Vec<TableId>,
}

impl Build for InOrder {
    type Product = Hour;

    fn tables(&self) -> Option<Tables> {
        None
    }

    fn build(&self, frame: &DecodedFrame, _seen: &Seen) -> io::Result<Hour> {
        for table in &self.reads {
            if let TableView::Unavailable { reason } = &*frame.tables[table.index()] {
                return Err(io::Error::other(reason.clone()));
            }
        }
        Ok(frame.hour)
    }
}

fn vision_content() -> Arc<vision::Content> {
    use viewer_foundation::map::visibility::{FogContent, Topology, test_defines};
    let neighbors = BTreeMap::from([(1, vec![2]), (2, vec![1])]);
    Arc::new(vision::Content::new(Ok(FogContent { topology: Topology { land: BTreeSet::from([1]), sea: BTreeSet::from([2]), neighbors }, defines: test_defines() })))
}

/// Forty hours of the vision layer's tables and the Clock: three countries with their tags, so
/// typed rows of Countries can be made, the viewer the first.
fn record() -> Units {
    recording(40, |_, _| {})
}

/// `hours` of [`record`]'s tables with the Clock's speed written anew each hour: the Clock changes
/// every hour, LocalPlayer never.
fn ticking(hours: u32) -> Units {
    recording(hours, |at, store| {
        if at > 0 {
            store.table(TableId::Clock.index()).unwrap().write(0, recorded::column(TableId::Clock, "speed"), Cell::I32(at as i32 + 1)).unwrap();
        }
    })
}

fn recording(hours: u32, mut every: impl FnMut(u32, &mut Store)) -> Units {
    let mut tables: Vec<TableId> = VISION_TABLES.to_vec();
    tables.push(TableId::Clock);
    tables.sort_unstable();
    tables.dedup();
    // Each test its own recording: one name is one playhead, and tests run side by side.
    static RECORDED: AtomicUsize = AtomicUsize::new(0);
    let name = format!("rebuilt test {}", RECORDED.fetch_add(1, Ordering::Relaxed));
    let strings = |at: u32| if at == 0 { ["AAA", "BBB", "CCC"].map(str::to_owned).to_vec() } else { Vec::new() };
    let pages = crate::layers::rails::tests::recorded(hours, strings, |at, store: &mut Store| {
        let column = recorded::column;
        if at == 0 {
            for (slot, raw) in [101u32, 102, 103].into_iter().enumerate() {
                let slot = slot as u32;
                store.table(TableId::CountryLinks.index()).unwrap().insert_at(slot, u128::from(raw)).unwrap();
                store.table(TableId::CountryLinks.index()).unwrap().write(slot, column(TableId::CountryLinks, "canonical_slot"), Cell::U32(raw - 100)).unwrap();
                store.table(TableId::Countries.index()).unwrap().insert_at(slot, u128::from(raw)).unwrap();
                store.table(TableId::Countries.index()).unwrap().write(slot, column(TableId::Countries, "canonical_slot"), Cell::U32(raw - 100)).unwrap();
                store.table(TableId::Countries.index()).unwrap().write(slot, column(TableId::Countries, "tag"), Cell::Bytes(slot)).unwrap();
            }
            store.table(TableId::LocalPlayer.index()).unwrap().insert_at(0, 0).unwrap();
            store.table(TableId::LocalPlayer.index()).unwrap().write(0, column(TableId::LocalPlayer, "country"), Cell::U32(101)).unwrap();
            store.table(TableId::Clock.index()).unwrap().insert_at(0, 0).unwrap();
            store.table(TableId::Clock.index()).unwrap().write(0, column(TableId::Clock, "speed"), Cell::I32(1)).unwrap();
        }
        every(at, store);
        for &table in &tables {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
    });
    let kinds = Arc::new((0..TableId::ALL.len()).map(|table| pages.kinds(table).map(<[Kind]>::to_vec).unwrap_or_default()).collect());
    let timeline = Timeline::whole(&*pages).unwrap();
    Units { name, pages, timeline, kinds }
}

/// `units` with nothing of `table` in any block's page: a recording that never held it.
pub(crate) fn without(units: Units, table: TableId) -> Units {
    struct Without {
        inner: Arc<dyn Pages>,
        runs: Vec<Run>,
    }
    impl Pages for Without {
        fn extents(&self, run: usize) -> &dyn Extents {
            self.inner.extents(run)
        }
        fn runs(&self) -> &[Run] {
            &self.runs
        }
        fn kinds(&self, table: usize) -> Option<&[Kind]> {
            self.inner.kinds(table)
        }
    }
    let runs = units
        .pages
        .runs()
        .iter()
        .map(|run| {
            let mut pages = run.pages.clone();
            for page in &mut pages {
                if let Some(entry) = page.tables.get_mut(table.index()) {
                    *entry = TableEntry::default();
                }
            }
            Run::new(run.first_hour, pages).unwrap()
        })
        .collect();
    let pages: Arc<dyn Pages> = Arc::new(Without { inner: units.pages, runs });
    let timeline = Timeline::whole(&*pages).unwrap();
    Units { name: units.name, pages, timeline, kinds: units.kinds }
}

/// The vision layer, then each of `probes` declaring only the Clock, each given the playhead's rows
/// before the seek to the first hour (so the seek brings their tables), played until that reset is
/// swapped in.
fn side_with<B: Build>(units: Units, probes: Vec<(&'static str, B)>) -> MapSide {
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0 }).unwrap();
    let mut layers: Vec<Box<dyn Layer>> = vec![Box::new(vision::Vision::new(vision_content()))];
    for (name, probe) in probes {
        layers.push(Box::new(Whole::new(name, probe, |_, _| true, |_| 0, &[TableId::Clock], Cadence::EveryStep, false, false).unwrap()));
    }
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    assert!(side.refusals().next().is_none());
    for at in 1..side.parts().unwrap().1.len() {
        layer_of::<B>(&side, at).rebuilt().set_source(Arc::clone(&entry), Recording { branch: 1 }, (Choice::default(), Some(Arc::new(Vision::default())))).unwrap();
    }
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    recorded::frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side
}

fn side_of(units: Units, reads: Vec<TableId>, gate: &Arc<Gate>) -> MapSide {
    side_with(units, vec![("probe", Probe { reads, gate: Arc::clone(gate) })])
}

fn layer_of<B: Build>(side: &MapSide, at: usize) -> &Whole<B> {
    side.parts().unwrap().1[at].any().downcast_ref::<Whole<B>>().unwrap()
}

fn probe_of(side: &MapSide) -> &Whole<Probe> {
    layer_of(side, 1)
}

#[test]
fn a_rebuilt_layer_after_vision_settles_while_its_worker_holds_its_build() {
    let gate = Gate::shut();
    let mut side = side_of(record(), Vec::new(), &gate);
    let _opens = Opens(Arc::clone(&gate));
    recorded::frames_until(&mut side, |_, _| gate.reached());
    // The proof's settle: settled until no layer carries anything, while the worker still builds.
    let mut carrying = Vec::new();
    for _ in 0..1_000 {
        side.settle_all().unwrap();
        carrying = side.parts().unwrap().1.iter().filter(|layer| layer.carried().entities > 0).map(|layer| (layer.name(), layer.carried().entities)).collect();
        if carrying.is_empty() {
            break;
        }
    }
    assert!(carrying.is_empty(), "the layers carry {carrying:?} while the probe's worker holds its build");
    assert_eq!(probe_of(&side).published(), Published::Nothing, "before its first product it has published nothing");
    gate.open();
    let hour = side.applied().expect("the head has reached an hour");
    let built = probe_of(&side).rebuilt().wait(hour, Duration::from_secs(30)).unwrap();
    assert_eq!(built.product.as_ref().ok(), Some(&hour));
    side.settle_all().unwrap();
    assert_eq!(probe_of(&side).published(), Published::Cadence(hour), "once built it shows the hour of its rows");
    assert_eq!(probe_of(&side).carried().entities, 0);
}

#[test]
fn a_table_the_recording_holds_nothing_of_is_refused_by_name_and_never_declared() {
    let gate = Gate::opened();
    let mut side = side_of(without(record(), TableId::Factions), vec![TableId::Factions], &gate);
    side.settle_all().unwrap();
    let hour = side.applied().expect("the head has reached an hour");
    let built = probe_of(&side).rebuilt().wait(hour, Duration::from_secs(30)).unwrap();
    let why = built.product.as_ref().expect_err("the product is refused");
    assert!(why.contains("holds no Factions"), "the refusal names the table: {why}");
    assert!(!probe_of(&side).rebuilt().tables().contains(&TableId::Factions), "the table is never declared");
    assert!(side.entry().playhead().error().is_none(), "the playhead goes on");
    assert_eq!(probe_of(&side).carried().entities, 0);
}

#[test]
fn a_build_runs_under_the_recordings_executable_tables_and_one_that_panics_is_refused_by_name() {
    let mut side = side_with(record(), vec![("scoped", NeedsTables { given: Some(recording_tables()) }), ("unscoped", NeedsTables { given: None })]);
    side.settle_all().unwrap();
    let hour = side.applied().expect("the head has reached an hour");
    let scoped = layer_of::<NeedsTables>(&side, 1).rebuilt().wait(hour, Duration::from_secs(30)).unwrap();
    assert_eq!(scoped.product.as_ref().ok(), Some(&hour), "the build ran under the recording's tables: {:?}", scoped.product.as_ref().err());
    let unscoped = layer_of::<NeedsTables>(&side, 2).rebuilt();
    let refused = unscoped.wait(hour, Duration::from_secs(30)).unwrap();
    let why = refused.product.as_ref().expect_err("a build that panics is refused");
    assert!(why.contains("panicked") && why.contains("no recording's tables are in scope"), "the refusal names the panic: {why}");
    let builds = unscoped.builds().count;
    // The worker goes on: a later frame is built, and refused, again.
    recorded::frames_until(&mut side, |side, _| layer_of::<NeedsTables>(side, 2).rebuilt().builds().count > builds);
    let unscoped = layer_of::<NeedsTables>(&side, 2).rebuilt();
    assert!(unscoped.stopped().is_none(), "the worker has not stopped");
    assert!(unscoped.newer(None).expect("a product").product.is_err());
    assert!(side.entry().playhead().error().is_none(), "the playhead goes on");
}

#[test]
fn a_build_that_misses_three_tables_learns_the_three_in_one_round() {
    // Three tables the recording holds whole and empty: typed rows of them can be made (its
    // Countries rows lack most columns, which rows at the head rightly refuse by name).
    let reads = vec![TableId::FactionMembers, TableId::WarRelations, TableId::Subjects];
    let mut side = side_with(record(), vec![("in order", InOrder { reads: reads.clone() })]);
    recorded::frames_until(&mut side, |side, _| layer_of::<InOrder>(side, 1).rebuilt().builds().count > 0);
    let rebuilt = layer_of::<InOrder>(&side, 1).rebuilt();
    let tables = rebuilt.tables();
    assert!(reads.iter().all(|table| tables.contains(table)), "every table the build reaches is declared: {tables:?}");
    assert_eq!(rebuilt.builds().discovered, 1, "one round learned the three");
    let built = rebuilt.newer(None).expect("a product");
    assert_eq!(built.product.as_ref().ok(), Some(&built.hour), "once joined the build reads all three: {:?}", built.product.as_ref().err());
}

/// The vision layer, then the probe as the window makes it: its rows from the playhead, built for the
/// view's perspective with the vision left to the builder, each product it takes handed to the scene;
/// drawn below a camera height of 500.
fn window_side(units: Units, gate: &Arc<Gate>) -> MapSide {
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0 }).unwrap();
    let probe = Whole::new("probe", Probe { reads: Vec::new(), gate: Arc::clone(gate) }, |_, view| view.camera_height < 500.0, |_| 0, &[TableId::Clock], Cadence::EveryStep, false, false).unwrap().in_window(Arc::clone(&entry), Recording { branch: 1 }).unwrap();
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(vision::Vision::new(vision_content())), Box::new(probe)];
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    recorded::frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side
}

/// What a wait of the window's tests saw last: the map's hour and compose, and the probe's
/// published, carried, product, builds and the tables its worker waits for.
fn window_state(side: &MapSide, gate: &Gate) -> String {
    let probe = side.parts().map(|_| probe_of(side)).map(|probe| {
        let rebuilt = probe.rebuilt();
        format!("published {:?}, carried {:?}, shows {}, product at {:?}, builds {}, tables {:?}, awaiting {:?}, stopped {:?}", probe.published(), probe.carried(), probe.shows(), probe.product().map(|built| built.hour), rebuilt.builds().count, rebuilt.tables(), rebuilt.awaiting(), rebuilt.stopped())
    });
    format!("applied {:?}, newest {:?}, composing {}, gate reached {}; the probe: {probe:?}", side.applied(), side.entry().playhead().newest(), side.composing(), gate.reached())
}

/// Frames with the head at `head` (none: the hour applied) until `done` holds; fails by name,
/// printing what it saw, within 120 s.
fn frames_at(side: &mut MapSide, head: Option<Hour>, gate: &Gate, what: &str, mut done: impl FnMut(&MapSide) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let at = head.or(side.applied()).unwrap_or(0);
        let frame = side.frame(at).unwrap();
        if done(side) {
            return;
        }
        assert!(Instant::now() < deadline, "{what} not within 120 s with the head at {at}: {}", window_state(side, gate));
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
}

/// The head moves only when the test moves it: held at the hour applied while the worker builds,
/// then one step on with the layer hidden.
#[test]
fn a_window_layer_shows_todays_plan_until_its_first_product_and_nothing_where_the_view_hides_it() {
    let gate = Gate::shut();
    let mut side = window_side(record(), &gate);
    let _opens = Opens(Arc::clone(&gate));
    frames_at(&mut side, None, &gate, "the probe's worker reaching its build", |_| gate.reached());
    assert!(!probe_of(&side).shows(), "until its first product the window draws today's plan");
    assert!(probe_of(&side).uploaded(), "and the layer hands the scene nothing");
    gate.open();
    let hour = side.applied().expect("the head has reached an hour");
    let built = probe_of(&side).rebuilt().wait(hour, Duration::from_secs(120)).unwrap();
    assert_eq!(built.product.as_ref().ok(), Some(&hour), "built for the view's perspective, its vision left to the builder");
    side.settle_all().unwrap();
    assert!(probe_of(&side).shows(), "with a product the window draws the layer's: today's makes none");
    assert!(!probe_of(&side).uploaded(), "the product it took is due to the scene");
    assert_eq!(probe_of(&side).refusal(), None);
    side.set_view(View { camera_height: 600.0, ..View::default() }).unwrap();
    let next = hour + 4;
    assert!(next < 40, "the recording holds hour {next}");
    frames_at(&mut side, Some(next), &gate, &format!("hour {next} applied with the probe hidden"), |side| side.applied() >= Some(next) && !side.composing());
    assert!(probe_of(&side).product().is_none(), "hidden, it holds no product");
    assert!(probe_of(&side).shows(), "hidden, it draws nothing, as today's draws nothing there");
    assert!(!probe_of(&side).uploaded(), "an empty plan is due to the scene");
    // Before any frame settles it again, so before its worker can build: no product yet.
    side.set_view(View::default()).unwrap();
    assert!(!probe_of(&side).shows(), "shown again, today's plan is drawn until the layer's first product");
}

/// Builds nothing of the rows, and stands at any rows that hold its tables unchanged.
struct Steady;

impl Build for Steady {
    type Product = ();

    fn tables(&self) -> Option<Tables> {
        None
    }

    fn build(&self, _frame: &DecodedFrame, _seen: &Seen) -> io::Result<()> {
        Ok(())
    }

    fn stands(&self, _product: &(), _frame: &DecodedFrame, _seen: &Seen) -> bool {
        true
    }
}

/// The vision layer, then [`Steady`] reading `reads` from rows at `cadence`; its reader declared
/// before the seek to the first hour (so the seek's reset brings its tables), played until that
/// reset is swapped in.
fn steady_side(units: Units, reads: &[TableId], cadence: Cadence) -> MapSide {
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0 }).unwrap();
    let steady = Whole::new("steady", Steady, |_, _| true, |_| 0, reads, cadence, false, false).unwrap();
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(vision::Vision::new(vision_content())), Box::new(steady)];
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    steady_of(&side).rebuilt().set_source(Arc::clone(&entry), Recording { branch: 1 }, (Choice::default(), None)).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    recorded::frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side
}

fn steady_of(side: &MapSide) -> &Whole<Steady> {
    layer_of(side, 1)
}

/// The frame that applies the next step, four hours on (the head moves only here, a step a frame):
/// the hour.
fn next_hour(side: &mut MapSide) -> i32 {
    let before = side.applied().expect("the head has reached an hour");
    for _ in 0..50_000_000u64 {
        let head = side.entry().playhead().newest().unwrap_or(before).min(before + 4);
        let frame = side.frame(head).unwrap();
        if let Some(hour) = frame.applied.filter(|&applied| applied != before) {
            return hour;
        }
        std::thread::yield_now();
    }
    panic!("no step past hour {before} was applied");
}

#[test]
fn a_layer_builds_nothing_while_no_table_it_reads_changes_and_its_product_stands_at_the_newer_hours() {
    let mut side = steady_side(ticking(200), &[TableId::LocalPlayer], Cadence::EveryStep);
    let mut last = 0;
    for _ in 0..40 {
        last = next_hour(&mut side);
        assert!(steady_of(&side).rebuilt().looked(last, Instant::now() + Duration::from_secs(30)), "the worker never looked at the rows of hour {last}");
    }
    let built = steady_of(&side).rebuilt().wait(last, Duration::from_secs(30)).unwrap();
    let builds = steady_of(&side).rebuilt().builds();
    assert_eq!(builds.count, 1, "the Clock changed every hour and LocalPlayer never: one build, the first ({builds:?})");
    assert!(builds.confirmed >= 39, "the product stood at each newer hour's rows: {builds:?}");
    assert_eq!(built.current(), last, "the product is current at the newest rows");
    side.settle_all().unwrap();
    assert_eq!(steady_of(&side).published(), Published::Cadence(last), "it shows the newest hour its product stands at");
    assert_eq!(steady_of(&side).carried().entities, 0);
}

/// The Clock changes every hour and a step is applied every frame: with rows due at the step, the
/// worker makes the product of each step's rows once the one before is made, held for no clock. On
/// the base it took rows at most once a tenth of a second of its pace's clock, at four or ten
/// frames of rows a second: a step's rows could be passed over for the next tick's.
#[test]
fn a_layer_with_a_step_every_frame_makes_the_product_of_each_steps_rows_and_waits_for_no_clock() {
    let mut side = steady_side(ticking(520), &[TableId::Clock], Cadence::Ready);
    let first = steady_of(&side).rebuilt().builds().count;
    let steps = 60u64;
    let mut last = 0;
    for _ in 0..steps {
        last = next_hour(&mut side);
        let built = steady_of(&side).rebuilt().wait(last, Duration::from_secs(30)).unwrap();
        assert_eq!(built.hour, last, "made from the rows of the step applied");
    }
    let builds = steady_of(&side).rebuilt().builds();
    assert!(builds.count - first >= steps, "a product for each step's rows: {builds:?}");
    side.settle_all().unwrap();
    assert_eq!(steady_of(&side).published(), Published::Cadence(last));
}
