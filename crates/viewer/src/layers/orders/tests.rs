use super::{Orders, Plans};
use crate::{
    layers::{
        Layer,
        recorded::{self, column},
    },
    playhead_host::{
        open,
        rows::{Recording, Rows},
    },
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
    let entry = open(recorded::record("orders", 60, campaign), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0 }).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, Recording { branch: 0 }).unwrap();
    let mut layer = Orders::new(Arc::new(Links));
    layer.attach(Arc::clone(&rows));
    layer.set_vision(Choice::default(), Arc::new(Vision::sharing(3, [])));
    rows.settled();
    rows.drive(0);
    entry.playhead().seek(0);
    // At the newest step built, as a map's frames would take it, until `to` is reached.
    let through = |layer: &mut Orders, to: i32| loop {
        let head = entry.playhead().newest().unwrap_or(0);
        rows.drive(head);
        layer.settle_at(head, LIMIT).unwrap();
        if head >= to {
            break;
        }
    };

    through(&mut layer, 36);
    let (hour, plan) = layer.plan().expect("a plan from the first rows");
    assert_eq!((hour, plan.report.recorded), (0, 1));
    assert_eq!(layer.marked()[..2], [1, 0], "one plan while the rows are unchanged, no table learned");

    through(&mut layer, 55);
    let (hour, plan) = layer.plan().unwrap();
    assert_eq!(plan.report.recorded, 2);
    assert!((45..50).contains(&hour), "made again from the rows of the step the table changed in, at {hour}");
    assert_eq!(layer.marked()[0], 2);
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
    let entry = open(recorded::record("orders notes", 40, ordering), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0 }).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, Recording { branch: 0 }).unwrap();
    let mut layer = Orders::new(Arc::new(Links));
    layer.attach(Arc::clone(&rows));
    layer.set_vision(Choice::default(), Arc::new(Vision::sharing(0, [])));
    rows.settled();
    rows.drive(0);
    entry.playhead().seek(0);
    let through = |layer: &mut Orders, to: i32| loop {
        let head = entry.playhead().newest().unwrap_or(0);
        rows.drive(head);
        layer.settle_at(head, LIMIT).unwrap();
        if head >= to {
            break;
        }
    };

    through(&mut layer, 12);
    assert!(layer.plan().is_some_and(|(hour, _)| hour < 20), "a plan from rows before the order");
    assert!(layer.notes().is_empty(), "no order of the viewing country, no note: {:?}", layer.notes());

    through(&mut layer, 30);
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
    let entry = open(recorded::record("orders driven past the seek", 40, ordering), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0 }).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, Recording { branch: 0 }).unwrap();
    let mut layer = Orders::new(Arc::new(Links));
    layer.attach(Arc::clone(&rows));
    layer.set_vision(Choice::default(), Arc::new(Vision::sharing(0, [])));
    rows.settled();
    rows.drive(0);
    entry.playhead().seek(0);
    let through = |layer: &mut Orders, to: i32| loop {
        let head = entry.playhead().newest().unwrap_or(0);
        rows.drive(head);
        layer.settle_at(head, LIMIT).unwrap();
        if head >= to {
            break;
        }
    };
    through(&mut layer, 30);
    let old = layer.plan().unwrap().0;
    assert!(old >= 20, "a plan from the rows holding the order, at {old}");

    entry.playhead().seek(4);
    layer.compose(&crate::playhead_host::mirror::Mirrors::default()).unwrap();
    rows.drive(4);
    layer.settle_at(4, LIMIT).unwrap();
    assert_eq!(layer.plan().unwrap().0, 4, "the plan of the rows at the sought hour");
}

/// A seek back with the steps past the sought hour built before the map's next frame: the plan
/// a whole settle shows at the sought hour is the plan of the rows at that hour. The rows worker
/// is held (a signal) while the playhead builds those steps, and released before the map takes
/// the seek's reset; on the head it followed the map's last head, of the seek before, took the
/// new seek's steps through it, and the settle showed the plan of a later hour (`orders:today`
/// differing after a seek in the units equality jobs).
#[test]
fn after_a_seek_back_the_plan_shown_at_the_sought_hour_is_made_from_its_rows() {
    let entry = open(recorded::record("orders after a seek back", 40, ordering), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0 }).unwrap();
    entry.set_capture(CaptureMode::Full);
    let mut side = crate::playhead_host::MapSide::new(Arc::clone(&entry), Vec::new(), crate::layers::View::default()).unwrap();
    let rows = Rows::of(&entry, Recording { branch: 0 }).unwrap();
    let mut layer = Orders::new(Arc::new(Links));
    layer.attach(Arc::clone(&rows));
    layer.set_vision(Choice::default(), Arc::new(Vision::sharing(0, [])));
    rows.settled();
    side.anchor(0, false, 1.0, true);
    side.catch_up(0).unwrap();
    layer.settle_at(0, LIMIT).unwrap();
    side.catch_up(30).unwrap();
    layer.settle_at(30, LIMIT).unwrap();
    assert!(layer.plan().is_some(), "a plan before the seek");

    let held = rows.hold_before_take();
    let until = std::time::Instant::now() + LIMIT;
    assert!(held.holding(until), "the rows worker held before it takes");
    side.anchor(4, false, 1.0, true);
    layer.compose(&crate::playhead_host::mirror::Mirrors::default()).unwrap();
    // A step past the sought hour built before the worker wakes again (the hang guard only bounds
    // a stepper that never builds one).
    while entry.playhead().newest().is_none_or(|newest| newest <= 4) {
        assert!(std::time::Instant::now() < until, "the steps past the sought hour were never built");
        std::thread::yield_now();
    }
    let wakes = rows.wakes();
    held.release();
    assert!(rows.woke_past(wakes, until), "the rows worker woke once more before the map's next frame");
    side.catch_up(4).unwrap();
    layer.settle_at(4, LIMIT).unwrap();
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

/// A plan the test holds on the worker until it opens the gate (a minute at most, after which the
/// build goes on and the test's assertions fail by name), counting the builds.
#[derive(Default)]
struct Gated {
    open: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
    builds: std::sync::atomic::AtomicUsize,
}

impl Gated {
    fn set(&self, open: bool) {
        *self.open.lock().unwrap() = open;
        self.changed.notify_all();
    }

    fn builds(&self) -> usize {
        self.builds.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Plans for Gated {
    fn build(&self, frame: &DecodedFrame, ours: u32, show_allied: bool) -> io::Result<OrderField> {
        let until = std::time::Instant::now() + LIMIT;
        let mut open = self.open.lock().unwrap();
        while !*open {
            let Some(left) = until.checked_duration_since(std::time::Instant::now()) else { break };
            open = self.changed.wait_timeout(open, left).unwrap().0;
        }
        drop(open);
        self.builds.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Links.build(frame, ours, show_allied)
    }
}

/// A layer over its own recording of `name`, given a vision, its rows at hour 0.
fn gated(name: &str) -> (Arc<crate::playhead_host::Entry>, Arc<Rows>, Orders, Arc<Gated>) {
    let entry = open(recorded::record(name, 60, campaign), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 1.0, rate: 1.0 }).unwrap();
    entry.set_capture(CaptureMode::Full);
    let rows = Rows::of(&entry, Recording { branch: 0 }).unwrap();
    let plans = Arc::new(Gated::default());
    plans.set(true);
    let mut layer = Orders::new(Arc::clone(&plans) as Arc<dyn Plans>);
    layer.attach(Arc::clone(&rows));
    layer.set_vision(Choice::default(), Arc::new(Vision::sharing(3, [])));
    layer.take_vision();
    rows.settled();
    rows.drive(0);
    entry.playhead().seek(0);
    (entry, rows, layer, plans)
}

/// One live frame at `hour`: the rows driven there and waited for, then what a playing frame does
/// (no wait for the worker).
fn live(rows: &Rows, layer: &mut Orders, hour: i32) {
    rows.drive(hour);
    let reader = layer.rows.get_mut().unwrap();
    reader.as_ref().unwrap().wait(|frame| frame.hour == hour, LIMIT).unwrap();
    layer.exchange(false);
}

/// Live frames until nothing is on the worker, within the limit.
fn answered(rows: &Rows, layer: &mut Orders, hour: i32) {
    let until = std::time::Instant::now() + LIMIT;
    while layer.in_flight > 0 {
        assert!(std::time::Instant::now() < until, "the worker's answers never brought the jobs in flight back to none: {} left", layer.in_flight);
        live(rows, layer, hour);
    }
}

#[test]
fn playing_the_plan_is_current_while_its_tables_are_unchanged_and_a_remake_on_the_worker_shows_the_hour_before() {
    use crate::layers::Published;
    let (_entry, rows, mut layer, plans) = gated("orders playing");
    live(&rows, &mut layer, 0);
    answered(&rows, &mut layer, 0);
    assert_eq!(layer.published(), Published::Current, "the first plan made");
    for hour in 1..45 {
        live(&rows, &mut layer, hour);
        assert_eq!((layer.published(), layer.carried().entities), (Published::Current, 0), "hour {hour}: unchanged tables leave the plan current, nothing carried");
    }
    plans.set(false);
    live(&rows, &mut layer, 45);
    assert_eq!(layer.published(), Published::At(44), "the remake of hour 45 is on the worker: the plan shown stands through 44");
    assert_eq!(layer.carried().entities, 0, "the worker's job is not the frame thread's work");
    plans.set(true);
    answered(&rows, &mut layer, 45);
    for hour in 46..50 {
        live(&rows, &mut layer, hour);
        assert_eq!((layer.published(), layer.carried().entities), (Published::Current, 0), "hour {hour}: current again");
    }
    assert_eq!(plans.builds(), 2, "one plan at the start, one for the one change");
    assert_eq!(layer.plan().map(|(hour, plan)| (hour, plan.report.recorded)), Some((45, 2)));
}

#[test]
fn jobs_the_worker_coalesced_leave_none_in_flight() {
    use crate::layers::Published;
    let (_entry, rows, mut layer, plans) = gated("orders coalesced");
    plans.set(false);
    live(&rows, &mut layer, 0);
    for _ in 0..2 {
        layer.sent = None;
        layer.exchange(false);
    }
    assert_eq!(layer.in_flight, 3, "three jobs sent while the worker is held");
    plans.set(true);
    answered(&rows, &mut layer, 0);
    assert_eq!(layer.published(), Published::Current, "every job answered, the newest plan is current");
    assert!(layer.plan().is_some());
}

/// Between its jobs the plan is of the newest rows: while the map has applied an hour past the one
/// those rows stand at, the layer says their hour, made on the rows' cadence, never current; once
/// the rows stand at the hour applied it is current. On the base it said current whenever no job
/// was out, whatever the age of its rows.
#[test]
fn between_its_jobs_a_plan_of_rows_behind_the_hour_applied_says_their_hour_and_never_current() {
    use crate::layers::Published;
    let (_entry, rows, mut layer, _plans) = gated("orders behind");
    live(&rows, &mut layer, 0);
    answered(&rows, &mut layer, 0);
    layer.head = Some(3);
    layer.exchange(false);
    assert_eq!(layer.in_flight, 0, "no job out");
    assert_eq!(layer.published(), Published::Cadence(0), "the map applied hour 3, the plan's rows stand at 0");
    live(&rows, &mut layer, 3);
    answered(&rows, &mut layer, 3);
    assert_eq!(layer.published(), Published::Current, "its rows stand at the hour applied");
}
