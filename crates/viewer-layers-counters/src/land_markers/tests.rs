use std::sync::Arc;
use super::{Front, LandMarkers, TABLES};
use viewer_layer_core::{layers::{Layer, View}, engagements::{Engagements, Index}, test_support::{recorded::frames_until, sound_recording::{put, quick, record, units}}, playhead_host::{MapSide, open}};
use crate::naval_counters::Admits;
use content_reader_map::movement::MovementMap;
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::rows::{Field, NativeValue, Opaque, TableId};
use viewer_counter_kit::combat_markers::{LandCombatDefines, MarkerOutcome};
use viewer_engagements::{engaged::Engaged, engagement_history::{Covered, History, LAND}};
use viewer_gpu::camera::Placement;
use viewer_foundation::perspective::{Choice, Country, Vision};

struct Bare;

impl Front for Bare {
    fn anchor(&self, _province: u32, _slot: u32) -> Option<[f32; 3]> {
        None
    }

    fn movement(&self) -> Option<&MovementMap> {
        None
    }

    fn defines(&self) -> LandCombatDefines {
        LandCombatDefines::FIXTURE
    }

    fn cutoff(&self) -> f32 {
        f32::INFINITY
    }

    fn admits(&self, _placement: Placement, _viewport: [u32; 2]) -> Option<Admits> {
        None
    }
}

fn opaque<T: NativeValue>(value: T) -> Vec<u8> {
    let Field::Opaque(Opaque::Native(value)) = value.field() else { panic!("an opaque value") };
    let mut bytes = Vec::new();
    value.encode_value(&mut bytes).unwrap();
    bytes
}

fn statuses(store: &mut Store, at: u32, status: &impl Fn(u32, TableId) -> Status) {
    for table in TABLES.into_iter().chain([TableId::NavalCombats, TableId::NavalCombatDetails, TableId::Countries]) {
        store.status(table.index(), status(at, table), true, Some(at as i32), true).unwrap();
    }
}

fn division(store: &mut Store, slot: u32, id: u64, owner: u32, province: u32, combat_role: u8) {
    put(store, TableId::Divisions, slot, Some(u128::from(id)), &[("roster_owner", Cell::U32(owner)), ("province", Cell::U32(province)), ("hit_points", Cell::I64(50)), ("organisation", Cell::I64(30))]);
    put(store, TableId::DivisionMovement, slot, Some(u128::from(id)), &[("map_entity", Cell::U8(1)), ("combat_role", Cell::U8(combat_role)), ("path", Cell::U32List(&[])), ("alternate_path", Cell::U32List(&[]))]);
}

fn combat(store: &mut Store, slot: u32, id: u64, province: u32) {
    put(store, TableId::LandCombats, slot, Some(u128::from(id)), &[("province", Cell::U32(province)), ("attackers", Cell::U64List(&[2])), ("defenders", Cell::U64List(&[1]))]);
    put(
        store,
        TableId::LandCombatSides,
        slot,
        Some(u128::from(id)),
        &[("indicator", Cell::U8(1)), ("flags", Cell::Opaque(&opaque([0u32, 0]))), ("attacker_primary", Cell::U32List(&[11])), ("defender_primary", Cell::U32List(&[10])), ("defender_secondary", Cell::U32List(&[]))],
    );
}

fn world(store: &mut Store) {
    for (slot, key) in [(0, 10u128), (1, 11)] {
        put(store, TableId::CountryLinks, slot, Some(key), &[("canonical_slot", Cell::U32(slot))]);
        put(store, TableId::Countries, slot, Some(key), &[]);
    }
    division(store, 0, 1, 10, 5, 0);
    division(store, 1, 2, 11, 6, 2);
    combat(store, 0, 100, 5);
}

fn campaign(name: &str, status: impl Fn(u32, TableId) -> Status, mut hour: impl FnMut(u32, &mut Store)) -> viewer_layer_core::playhead_host::units::Units {
    units(
        record(400, |at, store| {
            statuses(store, at, &status);
            if at == 0 {
                world(store);
            }
            hour(at, store);
        }),
        name,
    )
}

fn history() -> Arc<History> {
    let history = Arc::new(History::default());
    let mut covered = Covered::new(0);
    covered.last = 399;
    covered.viewers.push((0, 0));
    covered.fold(false).record(LAND, 0, &[((100, 0), Engaged::Yes), ((100, 1), Engaged::Yes), ((101, 0), Engaged::No), ((101, 1), Engaged::No)], &[]);
    history.branches.lock().unwrap().insert(0, covered);
    history
}

fn markers(history: &Arc<History>) -> LandMarkers {
    let layer = LandMarkers::new(Arc::new(Bare), Some(Index { history: Arc::clone(history), branch: 0 }));
    layer.set_vision(Choice { country: Country::Recorded, fog: false }, Arc::new(Vision::sharing(0, []).without_fog()));
    layer
}

fn side_with(units: viewer_layer_core::playhead_host::units::Units, layer: LandMarkers, history: &Arc<History>, view: View) -> MapSide {
    let entry = open(units, quick()).unwrap();
    let engagements = Engagements::new(Some(Index { history: Arc::clone(history), branch: 0 }), Some(0));
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(engagements), Box::new(layer)], view).unwrap();
    assert_eq!(side.refusals().count(), 0, "{:?}", side.refusals().collect::<Vec<_>>());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side
}

fn side_for(units: viewer_layer_core::playhead_host::units::Units) -> MapSide {
    let history = history();
    side_with(units, markers(&history), &history, View { fog: false, ..View::default() })
}

fn reach(side: &mut MapSide, hour: i32) {
    for _ in 0..50_000_000u64 {
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

fn layer(side: &MapSide) -> &LandMarkers {
    side.parts().unwrap().1[1].any().downcast_ref::<LandMarkers>().unwrap()
}

fn settled_equals_composed(side: &mut MapSide) {
    side.settle_all().unwrap();
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(side).differ(&fresh), (0, None));
}

fn outcome(side: &MapSide, id: u64) -> Option<MarkerOutcome> {
    layer(side).outcomes().find(|(combat, _, _)| *combat == id).map(|(_, _, outcome)| outcome.clone())
}

#[test]
fn a_combat_changed_in_a_step_is_marked_once_and_settles_to_the_composed_value() {
    let units = campaign("land once", |_, _| Status::Complete, |at, store| {
        if at == 200 {
            put(store, TableId::LandCombatSides, 0, None, &[("indicator", Cell::U8(0))]);
            put(store, TableId::Divisions, 0, None, &[("organisation", Cell::I64(20))]);
        }
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let first = outcome(&side, 100).unwrap();
    let marker = first.marker.clone().expect("a seen combat draws");
    assert!(marker.seen);
    assert_eq!(marker.frame, Some(1), "the indicator byte");
    assert!(first.gaps.iter().any(|gap| gap.contains("LandCombatDetails")), "no detail row: no tint, by name: {:?}", first.gaps);
    let marked = layer(&side).marked();
    reach(&mut side, 204);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).marked() - marked, 1, "its sides row and a member's organisation mark it once");
    settled_equals_composed(&mut side);
    let changed = outcome(&side, 100).unwrap().marker.unwrap();
    assert_eq!(changed.frame, Some(0));
    assert_ne!(format!("{changed:?}"), format!("{marker:?}"));
}

#[test]
fn a_combat_changed_twice_before_it_settles_shows_its_newest_values() {
    let units = campaign("land twice", |_, _| Status::Complete, |at, store| match at {
        200 => put(store, TableId::LandCombatSides, 0, None, &[("indicator", Cell::U8(0))]),
        202 => put(store, TableId::LandCombatSides, 0, None, &[("indicator", Cell::U8(2))]),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let settles = side.hold_settles();
    reach(&mut side, 204);
    viewer_layer_core::test_support::recorded::released_unit_by_unit(&mut side, settles);
    settled_equals_composed(&mut side);
    assert_eq!(outcome(&side, 100).unwrap().marker.unwrap().frame, Some(1), "the newest indicator, not zero");
}

#[test]
fn a_combat_deleted_and_another_inserted_in_its_slot_updates_its_marker() {
    let units = campaign("land slots", |_, _| Status::Complete, |at, store| match at {
        200 => {
            store.table(TableId::LandCombats.index()).unwrap().delete(0).unwrap();
            store.table(TableId::LandCombatSides.index()).unwrap().delete(0).unwrap();
        }
        201 => combat(store, 0, 101, 5),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 200);
    settled_equals_composed(&mut side);
    assert!(outcome(&side, 100).is_none(), "the ended combat has no marker");
    reach(&mut side, 201);
    settled_equals_composed(&mut side);
    let unseen = outcome(&side, 101).unwrap();
    assert!(unseen.marker.as_ref().is_some_and(|marker| !marker.seen), "{unseen:?}");
    assert_eq!(unseen.limits.len(), 1, "an unseen icon names where its frame came from");
}

#[test]
fn tables_not_wholly_recorded_draw_as_today_draws_them() {
    let units = campaign(
        "land partial",
        |at, table| match (at, table) {
            (240..=247, TableId::LandCombatSides) => Status::Truncated { missing: 1 },
            (300..=307, TableId::CountryLinks) => Status::NotRecorded,
            _ => Status::Complete,
        },
        |at, store| {
            if at == 240 {
                store.table(TableId::LandCombatSides.index()).unwrap().delete(0).unwrap();
            }
            if at == 248 {
                put(store, TableId::LandCombatSides, 0, Some(100), &[("indicator", Cell::U8(1)), ("flags", Cell::Opaque(&opaque([0u32, 0]))), ("attacker_primary", Cell::U32List(&[11])), ("defender_primary", Cell::U32List(&[10])), ("defender_secondary", Cell::U32List(&[]))]);
            }
        },
    );
    let mut side = side_for(units);
    reach(&mut side, 243);
    settled_equals_composed(&mut side);
    let gap = outcome(&side, 100).unwrap();
    assert!(gap.marker.is_none());
    assert!(gap.gaps.iter().any(|cause| cause.contains("truncated LandCombatSides")), "{gap:?}");
    reach(&mut side, 303);
    side.settle_all().unwrap();
    assert!(layer(&side).refused().is_some_and(|why| why.contains("country link")), "{:?}", layer(&side).refused());
    assert_eq!(layer(&side).outcomes().count(), 0, "a refused layer keeps nothing");
    reach(&mut side, 320);
    settled_equals_composed(&mut side);
    assert!(outcome(&side, 100).unwrap().marker.is_some());
}

#[test]
fn a_view_that_hides_the_layer_declares_nothing_and_shown_again_composes() {
    let units = campaign("land hidden", |_, _| Status::Complete, |at, store| {
        if at == 200 {
            put(store, TableId::LandCombatSides, 0, None, &[("indicator", Cell::U8(0))]);
            put(store, TableId::Divisions, 0, None, &[("organisation", Cell::I64(20))]);
        }
    });
    let mut hidden = View { fog: false, ..View::default() };
    hidden.overlay.land_combats = false;
    let kinds = replay_recording::schema::schema().unwrap();
    let mut inputs = viewer_layer_core::layers::Inputs::new(&kinds, "land_markers");
    markers(&history()).inputs(&hidden, &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "the land combat markers switched off declare nothing");
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.set_view(hidden).unwrap();
    hold(&mut side, 206);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).carried().entities, 0, "a view that draws nothing carries nothing, though steps changed its tables");
    assert_eq!(layer(&side).held(), markers(&history()).held(), "and holds what an empty layer holds");
    assert_eq!(layer(&side).outcomes().count(), 0, "nothing is kept while hidden");
    assert!(layer(&side).published().is_none(), "and the window is given no marker to draw");
    settled_equals_composed(&mut side);
    side.set_view(View { fog: false, ..View::default() }).unwrap();
    frames_until(&mut side, |side, frame| frame.steps > 0 && !side.composing() && layer(side).refused().is_none());
    settled_equals_composed(&mut side);
    assert!(outcome(&side, 100).is_some(), "shown again, it composes");
    let mut drawn: Vec<u64> = layer(&side).outcomes().filter_map(|(_, _, outcome)| outcome.marker.as_ref()).map(|marker| marker.id).collect();
    drawn.sort_unstable();
    let published = layer(&side).published().expect("shown again, the window is given its markers");
    assert_eq!(published.markers.iter().map(|marker| marker.id).collect::<Vec<_>>(), drawn, "the markers the window draws are the layer's");
}

#[test]
fn outside_the_view_a_change_stays_marked_and_inside_it_is_made() {
    let units = campaign("land camera", |_, _| Status::Complete, |at, store| {
        if at == 200 {
            put(store, TableId::LandCombatSides, 0, None, &[("indicator", Cell::U8(0))]);
        }
    });
    let history = history();
    let mut layer_out = markers(&history);
    layer_out.admit_only(|province| province == 9);
    let mut side = side_with(units, layer_out, &history, View { fog: false, ..View::default() });
    reach(&mut side, 150);
    side.settle_all().unwrap();
    assert!(outcome(&side, 100).is_none(), "province 5 is outside the view");
    reach(&mut side, 204);
    side.settle_all().unwrap();
    assert!(layer(&side).outside() > 0, "the change outside is one bit");
    settled_equals_composed(&mut side);
}

#[test]
fn steps_mark_their_combats_reading_no_row_and_a_combat_changed_in_several_is_made_once() {
    let units = campaign("land apply marks", |_, _| Status::Complete, |at, store| match at {
        200 => {
            put(store, TableId::LandCombatSides, 0, None, &[("indicator", Cell::U8(0))]);
            put(store, TableId::Divisions, 0, None, &[("organisation", Cell::I64(20))]);
        }
        204 => put(store, TableId::Divisions, 1, None, &[("organisation", Cell::I64(25))]),
        208 => put(store, TableId::LandCombatSides, 0, None, &[("indicator", Cell::U8(2))]),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let made = layer(&side).made();
    let settles = side.hold_settles();
    reach(&mut side, 208);
    assert_eq!(layer(&side).applied_reads(), 0, "apply marks; it reads no row");
    assert_eq!(layer(&side).made(), made, "nothing is made while the settles are held");
    viewer_layer_core::test_support::recorded::released_unit_by_unit(&mut side, settles);
    assert_eq!(layer(&side).made() - made, 1, "combat 100, changed in three steps, is made once");
    assert_eq!(outcome(&side, 100).unwrap().marker.unwrap().frame, Some(1), "from the newest indicator");
    settled_equals_composed(&mut side);
}

