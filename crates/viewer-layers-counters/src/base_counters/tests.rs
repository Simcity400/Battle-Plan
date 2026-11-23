use super::{BaseCounters, TABLES};
use viewer_layer_core::{layers::{Layer, View}, test_support::{recorded::{frames_until, frames_until_named}, sound_recording::{put, quick, record, units}}, playhead_host::{MapSide, open}};
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::rows::TableId;
use std::sync::Arc;
use viewer_foundation::perspective::{Choice, Country, Vision};

fn world(store: &mut Store) {
    put(store, TableId::CountryLinks, 0, Some(10), &[("canonical_slot", Cell::U32(0))]);
    put(store, TableId::Provinces, 0, Some(5), &[("controller", Cell::U32(10)), ("state", Cell::U32(1))]);
    put(store, TableId::Provinces, 1, Some(6), &[("controller", Cell::U32(0)), ("state", Cell::U32(1))]);
    put(store, TableId::AirBases, 0, Some(100), &[("state", Cell::OptionalU32(Some(1))), ("kind", Cell::I32(0)), ("capacity", Cell::I32(10))]);
    put(store, TableId::AirBaseRegistrations, 0, Some(100 << 32), &[("capacity_raw", Cell::I64(1)), ("wings", Cell::U64List(&[900]))]);
    put(store, TableId::AirWings, 0, Some(900), &[("owner", Cell::U32(10)), ("country_override", Cell::I32(0)), ("air_base", Cell::U64(100)), ("base_override", Cell::U64(0)), ("planes", Cell::I32(5))]);
    put(store, TableId::StateFlags, 0, Some(1), &[("demilitarized", Cell::U8(0))]);
}

fn campaign(name: &str, status: impl Fn(u32, TableId) -> Status, mut hour: impl FnMut(u32, &mut Store)) -> viewer_layer_core::playhead_host::units::Units {
    units(
        record(400, |at, store| {
            for table in TABLES {
                store.status(table.index(), status(at, table), true, Some(at as i32), true).unwrap();
            }
            if at == 0 {
                world(store);
            }
            hour(at, store);
        }),
        name,
    )
}

fn bases() -> BaseCounters {
    let layer = BaseCounters::new();
    layer.set_vision(Choice { country: Country::Recorded, fog: false }, Arc::new(Vision::sharing(0, []).without_fog()));
    layer
}

fn side_for(units: viewer_layer_core::playhead_host::units::Units) -> MapSide {
    let entry = open(units, quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(bases())], View { fog: false, ..View::default() }).unwrap();
    assert_eq!(side.refusals().count(), 0, "{:?}", side.refusals().collect::<Vec<_>>());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side
}

fn reach(side: &mut MapSide, hour: i32) {
    for _ in 0..50_000_000u64 {
        viewer_layer_core::test_support::recorded::reads_landed(side);
        let head = side.entry().playhead().newest().unwrap_or(0).min(hour + 3);
        let frame = side.frame(head).unwrap();
        if !side.composing() && frame.applied.is_some_and(|applied| applied >= hour) {
            return;
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    panic!("the map side never reached hour {hour}");
}

fn hold(side: &mut MapSide, hour: i32) {
    while side.entry().playhead().newest().is_none_or(|newest| newest < hour) {
        let head = side.entry().playhead().newest().unwrap_or(0).min(hour);
        side.frame(head).unwrap();
        std::thread::yield_now();
    }
    side.frame(hour).unwrap();
}

fn layer(side: &MapSide) -> &BaseCounters {
    side.parts().unwrap().1[0].any().downcast_ref::<BaseCounters>().unwrap()
}

fn settled_equals_composed(side: &mut MapSide) {
    side.settle_all().unwrap();
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(side).values(), fresh.values());
    assert_eq!(layer(side).refused(), fresh.refused());
}

#[test]
fn a_part_changed_in_a_step_is_made_again_and_equals_composed() {
    let units = campaign("bases once", |_, _| Status::Complete, |at, store| {
        if at == 200 {
            put(store, TableId::AirWings, 0, None, &[("planes", Cell::I32(9))]);
            put(store, TableId::Provinces, 1, None, &[("controller", Cell::U32(10))]);
        }
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let first = layer(&side).values().cloned().expect("the base counters draw");
    let site = first.wings.get(&(1, viewer_foundation::building_facts::AIR_BASE)).copied().expect("state 1's air base");
    assert_eq!((site.planes, site.capacity, site.shown, site.stationed), (5, 10, true, Some(true)));
    assert_eq!(first.controlled.get(&6), Some(&false));
    let made = layer(&side).made();
    reach(&mut side, 204);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).applied_reads(), 0, "its applies only mark: the wing and the province are read at the settle");
    assert_eq!(layer(&side).made() - made, 1, "a wing's planes remake the wings part only; a controller is a province");
    settled_equals_composed(&mut side);
    let changed = layer(&side).values().cloned().unwrap();
    assert_eq!(changed.wings[&(1, viewer_foundation::building_facts::AIR_BASE)].planes, 9);
    assert_eq!(changed.controlled.get(&6), Some(&true));
}

const MANY: u32 = 300;

fn many(store: &mut Store, count: u32) {
    for i in 0..count {
        let (base, state, wing) = (1000 + u64::from(i), i * 3 + 2, 5000 + u64::from(i));
        put(store, TableId::AirBases, i + 1, Some(u128::from(base)), &[("state", Cell::OptionalU32(Some(state))), ("kind", Cell::I32(0)), ("capacity", Cell::I32(10))]);
        put(store, TableId::AirBaseRegistrations, i + 1, Some(u128::from(base) << 32), &[("capacity_raw", Cell::I64(1)), ("wings", Cell::U64List(&[wing]))]);
        put(store, TableId::AirWings, i + 1, Some(u128::from(wing)), &[("owner", Cell::U32(10)), ("country_override", Cell::I32(0)), ("air_base", Cell::U64(base)), ("base_override", Cell::U64(0)), ("planes", Cell::I32(5))]);
        put(store, TableId::StateFlags, i + 1, Some(u128::from(state)), &[("demilitarized", Cell::U8(0))]);
        put(store, TableId::Provinces, i + 2, Some(u128::from(100 + i)), &[("controller", Cell::U32(10)), ("state", Cell::U32(state))]);
    }
}

fn one_wing_changes(name: &str, count: u32) -> viewer_layer_core::playhead_host::units::Units {
    campaign(name, |_, _| Status::Complete, move |at, store| {
        if at == 0 {
            many(store, count);
        }
        if at == 200 {
            put(store, TableId::AirWings, 1 + 40, None, &[("planes", Cell::I32(9))]);
        }
    })
}

#[test]
fn one_base_s_wings_copy_its_shard_alone_and_share_every_other_part_with_the_window_s_version() {
    let mut side = side_for(one_wing_changes("bases many", MANY));
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let window = Arc::clone(layer(&side).published().expect("the base counters draw"));
    let made = layer(&side).made();
    reach(&mut side, 204);
    side.settle_all().unwrap();
    let next = Arc::clone(layer(&side).published().unwrap());
    assert!(!Arc::ptr_eq(&window, &next), "a changed wing is a new version");
    let site = (40 * 3 + 2, viewer_foundation::building_facts::AIR_BASE);
    assert_eq!((window.wings[&site].planes, next.wings[&site].planes), (5, 9));
    assert_eq!(layer(&side).made() - made, 1, "the wing's one site is made again, nothing whole");
    assert!(Arc::ptr_eq(&window.radar_notes, &next.radar_notes), "the parts it does not touch are the window's");
    for (shared, shards, part) in [
        (next.controlled.shared_with(&window.controlled), window.controlled.shard_count(), "controlled"),
        (next.site_bases.shared_with(&window.site_bases), window.site_bases.shard_count(), "site bases"),
        (next.radar.shared_with(&window.radar), window.radar.shard_count(), "radar"),
    ] {
        assert_eq!(shared, (shards, shards), "every {part} shard is shared");
    }
    let shards = window.wings.shard_count();
    assert!(shards >= 10, "the sites fill {shards} shards");
    assert_eq!(next.wings.shared_with(&window.wings), (shards - 1, shards), "the changed site's shard alone is copied: at most 64 states' sites, not the world's");
    assert_eq!(next.wings.differing(&window.wings).collect::<Vec<_>>(), [site]);
    settled_equals_composed(&mut side);
}

#[test]
fn one_changed_wing_costs_the_same_rows_whatever_the_number_of_bases() {
    let cost = |count: u32| {
        let mut side = side_for(one_wing_changes(&format!("bases {count}"), count));
        reach(&mut side, 150);
        side.settle_all().unwrap();
        let (work, made) = (layer(&side).work(), layer(&side).made());
        reach(&mut side, 204);
        side.settle_all().unwrap();
        assert_eq!(layer(&side).values().unwrap().wings[&(122, viewer_foundation::building_facts::AIR_BASE)].planes, 9);
        settled_equals_composed(&mut side);
        (layer(&side).work() - work, layer(&side).made() - made)
    };
    let (few, world) = (cost(50), cost(400));
    assert_eq!(few, world, "the apply and settle of one wing's planes read the same rows with 50 bases and with 400");
    assert!(few.0 <= 8, "a handful of rows: the wing, its registration, its base's count ({few:?})");
}

#[test]
fn while_changes_are_carried_it_shows_the_hour_before_the_oldest() {
    use viewer_layer_core::layers::Published;
    let units = campaign("bases pending", |_, _| Status::Complete, |at, store| {
        if at == 0 {
            many(store, MANY);
        }
        if at == 200 {
            put(store, TableId::AirWings, 1 + 40, None, &[("planes", Cell::I32(9))]);
        }
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let (mirrors, _) = side.parts().unwrap();
    let mut held = layer(&side).fresh();
    held.compose(mirrors).unwrap();
    assert_eq!(Layer::published(&held), Published::Current);
    held.bases.insert(1040);
    held.pending = Some(180);
    assert_eq!(Layer::published(&held), Published::At(179), "the hour before the oldest change it carries, not the last time it carried nothing");
    held.settle(mirrors, viewer_layer_core::layers::Settling::Whole).unwrap();
    assert_eq!(Layer::published(&held), Published::Current, "everything in, it is current with what it was given");
}

#[test]
fn tables_not_wholly_recorded_draw_as_today_draws_them() {
    let units = campaign(
        "bases partial",
        |at, table| match (at, table) {
            (240..=247, TableId::AirBaseRegistrations) => Status::Truncated { missing: 1 },
            (300..=307, TableId::CountryLinks) => Status::NotRecorded,
            _ => Status::Complete,
        },
        |_, _| {},
    );
    let mut side = side_for(units);
    reach(&mut side, 243);
    settled_equals_composed(&mut side);
    let partial = layer(&side).values().cloned().unwrap();
    let site = partial.wings[&(1, viewer_foundation::building_facts::AIR_BASE)];
    assert_eq!((site.stationed, site.foreign), (Some(true), None), "a truncated census decides only our own row's base");
    reach(&mut side, 303);
    side.settle_all().unwrap();
    assert!(layer(&side).refused().is_some_and(|why| why.contains("country link")), "{:?}", layer(&side).refused());
    assert!(layer(&side).values().is_none(), "a refused layer keeps nothing");
    reach(&mut side, 320);
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).values().unwrap().wings[&(1, viewer_foundation::building_facts::AIR_BASE)].foreign, Some(false));
}

#[test]
fn a_view_that_hides_the_layer_declares_nothing_and_shown_again_composes() {
    let units = campaign("bases hidden", |_, _| Status::Complete, |at, store| {
        if at == 200 {
            put(store, TableId::AirWings, 0, None, &[("planes", Cell::I32(9))]);
            put(store, TableId::Provinces, 1, None, &[("controller", Cell::U32(10))]);
        }
    });
    let mut hidden = View { fog: false, ..View::default() };
    hidden.overlay.bases = false;
    let kinds = replay_recording::schema::schema().unwrap();
    let mut inputs = viewer_layer_core::layers::Inputs::new(&kinds, "base_counters");
    bases().inputs(&hidden, &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "the base icons switched off declare nothing");
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.set_view(hidden).unwrap();
    hold(&mut side, 206);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).carried().entities, 0, "a view that draws nothing carries nothing, though steps changed its tables");
    assert_eq!(layer(&side).held(), bases().held(), "and holds what an empty layer holds");
    assert!(layer(&side).values().is_none(), "nothing is kept while hidden");
    settled_equals_composed(&mut side);
    side.set_view(View { fog: false, ..View::default() }).unwrap();
    let waited = |side: &MapSide| match side.parts() {
        None => "the base counters shown again composed from their tables (a compose is under way)".to_owned(),
        Some((mirrors, layers)) => {
            let home: Vec<TableId> = TABLES.into_iter().filter(|table| mirrors.get(*table).is_some()).collect();
            let layer = layers[0].any().downcast_ref::<BaseCounters>().unwrap();
            format!("the base counters shown again composed from their tables: refused {:?}, values held {}, tables home {home:?} of {TABLES:?}", layer.refused(), layer.values().is_some())
        }
    };
    frames_until_named(&mut side, waited, |side, _| !side.composing() && side.parts().is_some_and(|(_, layers)| layers[0].any().downcast_ref::<BaseCounters>().is_some_and(|layer| layer.refused().is_none() && layer.values().is_some())));
    settled_equals_composed(&mut side);
    assert!(layer(&side).values().is_some(), "shown again, it composes");
}

#[test]
fn the_layer_declares_against_this_schema() {
    let kinds = replay_recording::schema::schema().unwrap();
    let mut layer = bases();
    let mut inputs = viewer_layer_core::layers::Inputs::new(&kinds, "base_counters");
    layer.inputs(&View::default(), &mut inputs).unwrap();
    assert_eq!(inputs.into_tables().len(), TABLES.len());
}

fn fed(side: &MapSide) -> &super::BaseCountersFed {
    side.parts().unwrap().1[0].any().downcast_ref::<super::BaseCountersFed>().unwrap()
}

#[test]
fn on_their_worker_the_base_counters_equal_composed_at_each_hour() {
    let units = campaign("bases fed", |_, _| Status::Complete, |at, store| {
        if at == 200 {
            put(store, TableId::AirWings, 0, None, &[("planes", Cell::I32(9))]);
            put(store, TableId::Provinces, 1, None, &[("controller", Cell::U32(10))]);
        }
        if at == 260 {
            put(store, TableId::Provinces, 1, None, &[("controller", Cell::U32(0))]);
        }
    });
    let entry = open(units, quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(super::BaseCountersFed::new(bases()))], View { fog: false, ..View::default() }).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    for (hour, planes, controlled) in [(150, 5, false), (204, 9, true), (264, 9, false)] {
        reach(&mut side, hour);
        side.settle_all().unwrap();
        assert_eq!(Layer::published(fed(&side)), viewer_layer_core::layers::Published::Current, "hour {hour}");
        let (mirrors, _) = side.parts().unwrap();
        let mut fresh = fed(&side).fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(fed(&side).values(), fresh.values(), "hour {hour}: the worker's values against those composed from the frame's mirrors");
        assert_eq!(fed(&side).refused(), fresh.refused());
        let values = fed(&side).values().expect("the base counters draw");
        assert_eq!(values.wings[&(1, viewer_foundation::building_facts::AIR_BASE)].planes, planes, "hour {hour}");
        assert_eq!(values.controlled.get(&6), Some(&controlled), "hour {hour}");
    }
}

#[test]
fn after_a_slower_pace_the_first_step_built_again_reaches_the_worker() {
    use std::time::{Duration, Instant};
    let units = campaign("bases pace", |_, _| Status::Complete, |at, store| {
        if at == 13 {
            put(store, TableId::AirWings, 0, None, &[("planes", Cell::I32(9))]);
        }
        if at == 15 {
            put(store, TableId::Provinces, 1, None, &[("controller", Cell::U32(10))]);
        }
    });
    let entry = open(units, quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(super::BaseCountersFed::new(bases()))], View { fog: false, ..View::default() }).unwrap();
    entry.playhead().seek(0);
    let deadline = Instant::now() + Duration::from_secs(120);
    let frame = |side: &mut MapSide, head: i32, what: &str| {
        assert!(Instant::now() < deadline, "{what} within 120 s (applied {:?}, newest {:?})", side.applied(), side.entry().playhead().newest());
        let frame = side.frame(head).unwrap();
        if frame.steps == 0 {
            std::thread::yield_now();
        }
        frame
    };
    while !(frame(&mut side, 0, "the first compose").swapped.is_some() && !side.composing()) {}
    while side.applied().unwrap() < 8 {
        let next = side.applied().unwrap() + 4;
        while entry.playhead().newest() < Some(next) {
            let applied = side.applied().unwrap();
            frame(&mut side, applied, "the next four-hour step");
        }
        frame(&mut side, next, "the next four-hour step");
    }
    let applied = side.applied().unwrap();
    while entry.playhead().newest() < Some(applied + 8) {
        frame(&mut side, applied, "the four-hour steps built past the head, lent");
    }
    frame(&mut side, applied, "the four-hour steps built past the head, lent");
    entry.playhead().pace(1.0);
    let rebuilt = |newest: Option<i32>| newest.is_some_and(|newest| newest > applied && newest < applied + 8);
    while entry.playhead().held().2.resteps == 0 || !rebuilt(entry.playhead().newest()) {
        assert!(Instant::now() < deadline, "the steps past the head were not built again within 120 s (newest {:?})", entry.playhead().newest());
        std::thread::yield_now();
    }
    let mut voided = 0;
    for hour in applied + 1..=applied + 8 {
        loop {
            let done = frame(&mut side, hour, "a step of one hour");
            voided += done.voided;
            if done.applied == Some(hour) {
                break;
            }
        }
        side.settle_all().unwrap();
        assert_eq!(Layer::published(fed(&side)), viewer_layer_core::layers::Published::Current, "hour {hour}: the layer shows the hour applied");
        let (mirrors, _) = side.parts().unwrap();
        let mut fresh = fed(&side).fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(fed(&side).values(), fresh.values(), "hour {hour}: the worker's values against those composed from the frame's mirrors");
        let values = fed(&side).values().expect("the base counters draw");
        assert_eq!(values.wings[&(1, viewer_foundation::building_facts::AIR_BASE)].planes, if hour >= 13 { 9 } else { 5 }, "hour {hour}");
    }
    assert!(voided >= 1, "the four-hour steps lent past the head were voided");
}
