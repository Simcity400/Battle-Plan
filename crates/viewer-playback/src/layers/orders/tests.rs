use super::{Orders, Plans};
use crate::{
    layers::{
        Layer,
        recorded::{self, column},
    },
    playhead_host::{open, views::apart::Apart},
};
use playhead::Settings;
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::{
    rows::{TableId, tables},
    scope::CaptureMode,
};
use replay_frame::frame::DecodedFrame;
use std::{io, sync::Arc, time::Duration};
use viewer_counter_products::map::orders;
use viewer_foundation::map::{
    perspective::{Choice, Vision},
    ribbon_content::OrderField,
};

const LIMIT: Duration = Duration::from_secs(60);

/// A plan that counts the links it reads.
struct Links;

impl Plans for Links {
    fn build(&self, frame: &DecodedFrame, _ours: u32, _show_allied: bool) -> io::Result<OrderField> {
        let mut plan = OrderField::default();
        plan.report.recorded = frame.complete::<tables::CountryLinks>()?.len();
        Ok(plan)
    }
}

fn link(store: &mut Store, slot: u32, key: u128) {
    let mut links = store.table(TableId::CountryLinks.index()).unwrap();
    links.insert_at(slot, key).unwrap();
    links.write(slot, column(TableId::CountryLinks, "canonical_slot"), Cell::U32(slot)).unwrap();
}

/// Every memo table recorded; one link at hour 0 and another at 45.
fn campaign(at: u32, store: &mut Store) {
    for table in orders::memo_tables(false) {
        store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    }
    match at {
        0 => link(store, 0, 7),
        45 => link(store, 1, 9),
        _ => {}
    }
}

#[test]
fn a_plan_is_kept_while_its_rows_are_unchanged_and_made_again_from_the_hour_one_changed() {
    let entry = open(recorded::record("orders", 60, campaign), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }).unwrap();
    entry.set_capture(CaptureMode::Full);
    let mut fed = Apart::new(&entry, 0).unwrap();
    let mut layer = Orders::new(Arc::new(Links));
    layer.set_views(fed.store()).unwrap();
    layer.set_vision(Choice::default(), Arc::new(Vision::sharing(3, [])));
    entry.playhead().seek(0);
    // At the newest step applied, as a map's frames would take it, until `to` is reached.
    let through = |fed: &mut Apart, layer: &mut Orders, to: i32| loop {
        let head = entry.playhead().newest().unwrap_or(0);
        assert!(fed.drive(head), "the store never reached hour {head}");
        layer.settle_at(head, crate::layers::Settling::Whole).unwrap();
        if head >= to {
            break;
        }
    };

    through(&mut fed, &mut layer, 36);
    let (hour, plan) = layer.plan().expect("a plan from the first rows");
    assert_eq!((hour, plan.report.recorded), (0, 1));
    assert_eq!(layer.marked()[..2], [1, 0], "one plan while the rows are unchanged, no table learned");

    through(&mut fed, &mut layer, 55);
    let (hour, plan) = layer.plan().unwrap();
    assert_eq!(plan.report.recorded, 2);
    assert!((45..50).contains(&hour), "made again from the rows of the step the table changed in, at {hour}");
    assert_eq!(layer.marked()[0], 2);
}

/// The links plan, which reads the countries too once two links are recorded: as an invasion's
/// triggers reach tables past the memo tables only once the plan holds one.
struct Reaching;

impl Plans for Reaching {
    fn build(&self, frame: &DecodedFrame, ours: u32, show_allied: bool) -> io::Result<OrderField> {
        let mut plan = Links.build(frame, ours, show_allied)?;
        if plan.report.recorded == 2 {
            plan.report.drawn = frame.complete::<tables::Countries>()?.len();
        }
        Ok(plan)
    }
}

/// [`campaign`] with the countries recorded, none of them a row.
fn campaign_with_countries(at: u32, store: &mut Store) {
    campaign(at, store);
    store.status(TableId::Countries.index(), Status::Complete, true, Some(at as i32), true).unwrap();
}

/// A whole settle at the hour a build first reads a table it had not declared waits for that
/// table's rows and builds again: the plan it shows is the plan of the hour. On the base it handed
/// back the plan of an earlier hour as soon as it declared the table (`orders:today` differing
/// on full13 in 1940, where invasion plans learned their trigger tables one round at a time).
#[test]
fn a_whole_settle_that_learns_a_table_shows_the_plan_of_its_hour_made_once_the_table_joined() {
    let entry = open(recorded::record("orders learning", 60, campaign_with_countries), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }).unwrap();
    entry.set_capture(CaptureMode::Full);
    let mut fed = Apart::new(&entry, 0).unwrap();
    let mut layer = Orders::new(Arc::new(Reaching));
    layer.set_views(fed.store()).unwrap();
    layer.set_vision(Choice::default(), Arc::new(Vision::sharing(3, [])));
    entry.playhead().seek(0);
    // A settle at the head, and frames going on coming for that same hour while the build learns
    // a table: the rows of a table newly declared are made by a step after it, which only another
    // frame makes.
    let through = |fed: &mut Apart, layer: &mut Orders, to: i32| loop {
        let head = entry.playhead().newest().unwrap_or(0);
        let limit = std::time::Instant::now() + LIMIT;
        loop {
            assert!(fed.drive(head), "the store never reached hour {head}");
            layer.settle_at(head, crate::layers::Settling::step()).unwrap();
            if layer.made_for() == Some(head) {
                break;
            }
            assert!(std::time::Instant::now() < limit, "the plan never stood at hour {head}: it stands at {:?}", layer.made_for());
        }
        if head >= to {
            break head;
        }
    };

    through(&mut fed, &mut layer, 36);
    assert_eq!(layer.marked()[1], 0, "no table learned while one link is recorded");

    let head = through(&mut fed, &mut layer, 45);
    let (hour, plan) = layer.plan().expect("a plan");
    assert_eq!(layer.marked()[1], 1, "the countries learned once, at the hour the second link came");
    assert_eq!(layer.made_for(), Some(head), "the plan stands for the rows at the hour settled, not for rows before the table was learned");
    assert!((45..=head).contains(&hour), "made from rows after the second link, at {hour}, settled at {head}");
    assert_eq!((plan.report.recorded, plan.report.drawn), (2, 0), "both links, and the countries read");
}

/// Every memo table recorded; the viewing country (slot 0) holds theatre 5 and its orders group 77
/// with a plan from hour 0, and the group's first order from hour 20. Each row is written whole,
/// as the recorder gives it: a cell an insert leaves unwritten is not recorded, and a row with one
/// its codec needs makes the table unavailable at the head. The group's name alone is left not
/// recorded, which its codec takes as a counted refusal.
fn ordering(at: u32, store: &mut Store) {
    for table in orders::memo_tables(false) {
        store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    }
    let write = |store: &mut Store, table: TableId, field: &str, cell: Cell<'_>| store.table(table.index()).unwrap().write(0, column(table, field), cell).unwrap();
    match at {
        0 => {
            link(store, 0, 7);
            store.table(TableId::LandTheatres.index()).unwrap().insert_at(0, 5).unwrap();
            for (field, cell) in [("owner", Cell::U32(0)), ("ordinal", Cell::U32(0)), ("volunteers_theatre", Cell::I32(0))] {
                write(store, TableId::LandTheatres, field, cell);
            }
            store.table(TableId::OrdersGroups.index()).unwrap().insert_at(0, 77).unwrap();
            let group = [
                ("country", Cell::U32(0)),
                ("is_army_group", Cell::U8(0)),
                ("motorization_level", Cell::U8(0)),
                ("theatre", Cell::U64(5)),
                ("theatre_ordinal", Cell::OptionalU32(Some(0))),
                ("theater_group", Cell::U64(0)),
                ("parent", Cell::U64(0)),
                ("leader", Cell::U64(0)),
                ("hq_provider", Cell::U64(0)),
            ];
            for (field, cell) in group {
                write(store, TableId::OrdersGroups, field, cell);
            }
            let name = store.intern(b"Army 1").unwrap();
            store.table(TableId::OrdersGroupPlans.index()).unwrap().insert_at(0, 77).unwrap();
            let plan = [
                ("red", Cell::U32(255)),
                ("green", Cell::U32(0)),
                ("blue", Cell::U32(0)),
                ("alpha", Cell::U32(255)),
                ("primary_orders", Cell::U32List(&[])),
                ("fallback_orders", Cell::U32List(&[])),
                ("name", Cell::Bytes(name)),
            ];
            for (field, cell) in plan {
                write(store, TableId::OrdersGroupPlans, field, cell);
            }
        }
        20 => {
            store.table(TableId::ArmyOrders.index()).unwrap().insert_at(0, 77 << 32 | 1).unwrap();
            let order = [
                ("country", Cell::U32(0)),
                ("order_type", Cell::I32(0)),
                ("active", Cell::U8(0)),
                ("paradrop_target", Cell::U32(0)),
                ("path", Cell::U32List(&[])),
                ("target_states", Cell::U32List(&[])),
            ];
            for (field, cell) in order {
                write(store, TableId::ArmyOrders, field, cell);
            }
        }
        _ => {}
    }
}

#[test]
fn the_battle_plans_notes_are_stated_from_the_rows_the_viewing_country_holds_an_order_in() {
    let entry = open(recorded::record("orders notes", 40, ordering), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }).unwrap();
    entry.set_capture(CaptureMode::Full);
    let mut fed = Apart::new(&entry, 0).unwrap();
    let mut layer = Orders::new(Arc::new(Links));
    layer.set_views(fed.store()).unwrap();
    layer.set_vision(Choice::default(), Arc::new(Vision::sharing(0, [])));
    entry.playhead().seek(0);
    let through = |fed: &mut Apart, layer: &mut Orders, to: i32| loop {
        let head = entry.playhead().newest().unwrap_or(0);
        assert!(fed.drive(head), "the store never reached hour {head}");
        layer.settle_at(head, crate::layers::Settling::Whole).unwrap();
        if head >= to {
            break;
        }
    };

    through(&mut fed, &mut layer, 12);
    assert!(layer.plan().is_some_and(|(hour, _)| hour < 20), "a plan from rows before the order");
    assert!(layer.notes().is_empty(), "no order of the viewing country, no note: {:?}", layer.notes());

    through(&mut fed, &mut layer, 30);
    let (hour, _) = layer.plan().unwrap();
    assert!(hour >= 20, "a plan from rows holding the order, at {hour}");
    let mut today = viewer_foundation::map::tables::Limitations::default();
    today.note_battle_plan(1);
    assert_eq!(layer.notes(), today.into_lines(), "today's two lines for the one order drawn");
    assert_eq!(layer.notes().len(), 2);
}

/// A caller that drives the head settles at the sought hour after a seek: the plan is made from
/// the rows at that hour (before, rows driven past it were taken as its answer).
#[test]
fn after_a_seek_a_driven_settle_shows_the_plan_of_the_sought_hours_rows() {
    let entry = open(recorded::record("orders driven past the seek", 40, ordering), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }).unwrap();
    entry.set_capture(CaptureMode::Full);
    let mut fed = Apart::new(&entry, 0).unwrap();
    let mut layer = Orders::new(Arc::new(Links));
    layer.set_views(fed.store()).unwrap();
    layer.set_vision(Choice::default(), Arc::new(Vision::sharing(0, [])));
    entry.playhead().seek(0);
    let through = |fed: &mut Apart, layer: &mut Orders, to: i32| loop {
        let head = entry.playhead().newest().unwrap_or(0);
        assert!(fed.drive(head), "the store never reached hour {head}");
        layer.settle_at(head, crate::layers::Settling::Whole).unwrap();
        if head >= to {
            break;
        }
    };
    through(&mut fed, &mut layer, 30);
    let old = layer.plan().unwrap().0;
    assert!(old >= 20, "a plan from the rows holding the order, at {old}");

    entry.playhead().seek(4);
    layer.compose(&crate::playhead_host::mirror::Mirrors::default()).unwrap();
    assert!(fed.drive(4), "the store never reached the hour sought");
    layer.settle_at(4, crate::layers::Settling::Whole).unwrap();
    assert_eq!(layer.plan().unwrap().0, 4, "the plan of the rows at the sought hour");
}

/// A seek back with the steps past the sought hour already built: the plan a whole settle shows at
/// the sought hour is the plan of the rows at that hour, not of the steps built past it. The rows
/// the plan reads are the map side's own, made of the mirrors its step left, so the hour sought is
/// the hour applied; the test lets the playhead build every step of the seek before asking the
/// store for the hour sought. On the base the rows followed the map's last head, of the seek
/// before, and took the new seek's steps through it (`orders:today` differing after a seek in the
/// units equality jobs).
#[test]
fn after_a_seek_back_the_plan_shown_at_the_sought_hour_is_made_from_its_rows() {
    let entry = open(recorded::record("orders after a seek back", 40, ordering), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }).unwrap();
    entry.set_capture(CaptureMode::Full);
    let mut fed = Apart::new(&entry, 0).unwrap();
    let mut layer = Orders::new(Arc::new(Links));
    layer.set_views(fed.store()).unwrap();
    layer.set_vision(Choice::default(), Arc::new(Vision::sharing(0, [])));
    entry.playhead().seek(0);
    assert!(fed.drive(30), "the store never reached hour 30");
    layer.settle_at(30, crate::layers::Settling::Whole).unwrap();
    assert!(layer.plan().is_some(), "a plan before the seek");

    let until = std::time::Instant::now() + LIMIT;
    entry.playhead().seek(4);
    layer.compose(&crate::playhead_host::mirror::Mirrors::default()).unwrap();
    // Every step of the seek built before the store is asked for the hour sought, so a plan made
    // from a step past it would be taken if the rows chased an hour of their own.
    while entry.playhead().newest().is_none_or(|newest| newest <= 4) {
        assert!(std::time::Instant::now() < until, "the steps past the sought hour were never built");
        std::thread::yield_now();
    }
    assert!(fed.drive(4), "the store never reached the hour sought");
    layer.settle_at(4, crate::layers::Settling::Whole).unwrap();
    assert_eq!(layer.plan().unwrap().0, 4, "the plan shown at the sought hour is made from the rows at it");
}

fn label(key: &str) -> viewer_foundation::map::name_draw::WorldName {
    viewer_foundation::map::name_draw::WorldName {
        key: key.to_owned(),
        strategic_provinces: None,
        text: "118 Divs - Army Group 2".to_owned(),
        pages: vec![2, 2],
        corners: vec![[3399.9587, 10.140796, 1761.204], [3400.3167, 9.99884, 1760.4884]],
        uvs: vec![[0.0, 0.0, 0.5, 0.5], [0.5, 0.5, 1.0, 1.0]],
        aabb: [3398.0, 1759.0, 3401.0, 1762.0],
        wrapped: false,
        font_size: 1.0,
        spaces: 0,
        alpha: 1.0,
    }
}

#[test]
fn two_plans_labels_differ_by_content_never_by_their_upload_generation() {
    use super::differ_labels;
    use viewer_foundation::map::name_draw::{Placements, next_generation};
    let placements = |country| Placements { country, warnings: Vec::new(), generation: next_generation(), near: None };
    let today = placements(vec![label("1:1"), label("1:2")]);
    let layer = placements(vec![label("1:1"), label("1:2")]);
    // Each plan made takes a new generation: the whole-value print the proof compared differs.
    assert_ne!(format!("{today:?}"), format!("{layer:?}"));
    assert_eq!(differ_labels(&today, &layer), None);

    let mut moved = label("1:2");
    moved.corners[1][2] = 1760.5;
    let layer = placements(vec![label("1:1"), moved]);
    assert_eq!(differ_labels(&today, &layer).as_deref(), Some(r#"label "1:2" (at 1), corners[1]: today [3400.3167, 9.99884, 1760.4884], the layer [3400.3167, 9.99884, 1760.5]"#));

    let layer = placements(vec![label("1:2"), label("1:1")]);
    assert_eq!(differ_labels(&today, &layer).as_deref(), Some(r#"label 0: today "1:1", the layer "1:2""#));

    let layer = placements(vec![label("1:1")]);
    assert_eq!(differ_labels(&today, &layer).as_deref(), Some(r#"2 labels today, 1 the layer; the first unmatched today Some("1:2"), the layer None"#));
}

/// The gate's state: whether a build may go on, and the builds begun and finished. Every change
/// is announced on [`Gated::changed`], so a test waits for the worker's own word.
#[derive(Default)]
struct Gate {
    open: bool,
    begun: usize,
    builds: usize,
}

/// A plan the test holds on the worker until it opens the gate (a minute at most, after which the
/// build goes on and the test's assertions fail by name), counting the builds begun and finished.
#[derive(Default)]
struct Gated {
    gate: std::sync::Mutex<Gate>,
    changed: std::sync::Condvar,
}

impl Gated {
    fn gate(&self) -> std::sync::MutexGuard<'_, Gate> {
        self.gate.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set(&self, open: bool) {
        self.gate().open = open;
        self.changed.notify_all();
    }

    fn begun(&self) -> usize {
        self.gate().begun
    }

    fn builds(&self) -> usize {
        self.gate().builds
    }

    /// Waits for the worker to begin a build past `begun`, which it announces as it begins it;
    /// names `what` if none ever begins.
    fn begun_past(&self, begun: usize, what: &str) {
        let mut gate = self.gate();
        while gate.begun <= begun {
            let (held, waited) = self.changed.wait_timeout(gate, LIMIT).unwrap_or_else(std::sync::PoisonError::into_inner);
            gate = held;
            assert!(!waited.timed_out() || gate.begun > begun, "{what}");
        }
    }
}

impl Plans for Gated {
    fn build(&self, frame: &DecodedFrame, ours: u32, show_allied: bool) -> io::Result<OrderField> {
        let mut gate = self.gate();
        gate.begun += 1;
        self.changed.notify_all();
        while !gate.open {
            let (held, waited) = self.changed.wait_timeout(gate, LIMIT).unwrap_or_else(std::sync::PoisonError::into_inner);
            gate = held;
            if waited.timed_out() {
                break;
            }
        }
        gate.builds += 1;
        drop(gate);
        self.changed.notify_all();
        Links.build(frame, ours, show_allied)
    }
}

/// [`campaign`] with a country link inserted every hour: every hour changes a table the plan
/// reads, so every step's plan is a new build.
fn busy(at: u32, store: &mut Store) {
    for table in orders::memo_tables(false) {
        store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    }
    link(store, at, u128::from(at) + 7);
}

/// A layer over its own recording of `name` written by `written`, given a vision, its rows at hour 0.
fn gated(name: &str, written: fn(u32, &mut Store)) -> (Arc<crate::playhead_host::Entry>, Apart, Orders, Arc<Gated>) {
    let entry = open(recorded::record(name, 60, written), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0, ..Settings::default() }).unwrap();
    entry.set_capture(CaptureMode::Full);
    let fed = Apart::new(&entry, 0).unwrap();
    let plans = Arc::new(Gated::default());
    plans.set(true);
    let layer = Orders::new(Arc::clone(&plans) as Arc<dyn Plans>);
    layer.set_views(fed.store()).unwrap();
    layer.set_vision(Choice::default(), Arc::new(Vision::sharing(3, [])));
    let mut layer = layer;
    layer.take_vision();
    entry.playhead().seek(0);
    (entry, fed, layer, plans)
}

/// One live frame at `hour`: the step applied to the store's mirrors and the newest plan taken, as
/// a playing frame whose budget is already spent does it, with no wait for the worker at all.
fn live(fed: &mut Apart, layer: &mut Orders, hour: i32) {
    assert!(fed.drive(hour), "the store never reached hour {hour}");
    layer.head = Some(hour);
    layer.take();
}

/// Live frames at `hour` until the plan taken stands there, within the limit.
fn stands_at(fed: &mut Apart, layer: &mut Orders, hour: i32) {
    let limit = std::time::Instant::now() + LIMIT;
    while layer.made_for() != Some(hour) {
        assert!(std::time::Instant::now() < limit, "the plan never stood at hour {hour}: it stands at {:?}", layer.made_for());
        live(fed, layer, hour);
        std::thread::yield_now();
    }
}

#[test]
fn playing_the_plan_is_current_while_its_tables_are_unchanged_and_a_build_on_the_worker_shows_the_hour_before() {
    use crate::layers::Published;
    let (_entry, mut fed, mut layer, plans) = gated("orders playing", campaign);
    stands_at(&mut fed, &mut layer, 0);
    assert_eq!(layer.published(), Published::Current, "the first plan made");
    for hour in 1..45 {
        stands_at(&mut fed, &mut layer, hour);
        assert_eq!((layer.published(), layer.carried().entities), (Published::Current, 0), "hour {hour}: unchanged tables leave the plan current, nothing carried");
    }
    // The build of the hour the table changes in is held on the worker.
    let begun = plans.begun();
    plans.set(false);
    live(&mut fed, &mut layer, 45);
    plans.begun_past(begun, "the change at hour 45 began no build");
    live(&mut fed, &mut layer, 45);
    assert_eq!(layer.published(), Published::Cadence(44), "a build of hour 45 is on the worker: the plan shown stands through 44");
    assert_eq!(layer.plan().map(|(hour, plan)| (hour, plan.report.recorded)), Some((0, 1)), "the plan drawn is the one its rows were last made for");
    assert_eq!(layer.carried().entities, 0, "the worker's build is not the frame thread's work");
    plans.set(true);
    stands_at(&mut fed, &mut layer, 45);
    assert_eq!(layer.published(), Published::Current, "current again");
    for hour in 46..50 {
        stands_at(&mut fed, &mut layer, hour);
        assert_eq!((layer.published(), layer.carried().entities), (Published::Current, 0), "hour {hour}: current again");
    }
    assert_eq!(layer.plan().map(|(hour, plan)| (hour, plan.report.recorded)), Some((45, 2)));
}

/// Steps arriving faster than a build completes: the plan already finished is shown at its own
/// hour at every step, and the hour it stands at only moves forward. On the base the layer said it
/// had published nothing from the first step a build was in flight at to the last (it took the
/// hour its rows stood at only when no build was out, which at this pace it never was), and a
/// layer that publishes nothing freezes the whole map's drawn hour: the stress test showed one
/// hour for six seconds at a time.
#[test]
fn steps_faster_than_a_build_keep_the_plan_finished_shown_at_its_own_hour() {
    let (_entry, mut fed, mut layer, plans) = gated("orders behind a build", busy);
    stands_at(&mut fed, &mut layer, 0);
    let first = layer.made_for().expect("a first plan");
    // Every build from here waits on the gate, so no step's own plan is ever finished.
    plans.set(false);
    let mut stood = first;
    for hour in 1..40 {
        live(&mut fed, &mut layer, hour);
        let now = layer.made_for().unwrap_or_else(|| panic!("hour {hour}: the plan finished for hour {first} was not shown while a build was in flight"));
        assert!(now >= stood, "hour {hour}: the hour the plan stands at went back from {stood} to {now}");
        assert!(layer.plan().is_some(), "hour {hour}: the plan finished is still drawn");
        stood = now;
    }
    // Released, it catches up from the newest rows: it never builds a step of every hour it was
    // behind by, so it is at most one build behind the rows.
    plans.set(true);
    stands_at(&mut fed, &mut layer, 39);
    assert!(plans.builds() < 39, "it built every step it was behind by instead of the newest rows: {} builds", plans.builds());
    let (hour, plan) = layer.plan().expect("a plan");
    assert_eq!((hour, plan.report.recorded), (39, 40), "the plan of the rows at the hour it caught up to, holding a link an hour");
}

/// The plan of rows behind the hour applied says their hour, made on the rows' cadence, never
/// current: it neither holds the map's drawn hour nor is reported as standing at it.
#[test]
fn a_plan_of_rows_behind_the_hour_applied_says_their_hour_and_never_current() {
    use crate::layers::Published;
    let (_entry, mut fed, mut layer, _plans) = gated("orders cadence", campaign);
    stands_at(&mut fed, &mut layer, 0);
    layer.head = Some(3);
    layer.take();
    assert_eq!(layer.published(), Published::Cadence(0), "the map applied hour 3, the plan's rows stand at 0");
    stands_at(&mut fed, &mut layer, 3);
    assert_eq!(layer.published(), Published::Current, "its rows stand at the hour applied");
}

/// A compose is a new picture: the plan of the one before is still drawn, and reported as standing
/// nowhere until one is made from rows after the reset.
#[test]
fn a_compose_shows_the_plan_before_it_and_counts_none_until_one_is_made_after_the_reset() {
    use crate::layers::Published;
    let (_entry, mut fed, mut layer, _plans) = gated("orders composed", campaign);
    stands_at(&mut fed, &mut layer, 0);
    layer.compose(&crate::playhead_host::mirror::Mirrors::default()).unwrap();
    layer.take();
    assert_eq!(layer.published(), Published::Nothing, "no plan of this picture yet");
    assert!(layer.plan().is_some(), "the plan of the picture before is still drawn");
    stands_at(&mut fed, &mut layer, 0);
    assert_eq!(layer.published(), Published::Current);
}
