use std::sync::Arc;
use super::{NavalMarkers, Shore, TABLES};
use viewer_layer_core::{layers::{Layer, View}, engagements::{Engagements, Index}, test_support::{recorded::frames_until, sound_recording::{put, quick, record_runs, units}}, playhead_host::{MapSide, open}};
use content_reader_map::map::ProvinceKind;
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::rows::{Field, NativeValue, NavalCombatSideDetails, NavalProvinceCombatRow, Opaque, TableId};
use viewer_counters_naval::naval_markers::NavalMarkerValues;
use viewer_engagements::{engaged::Engaged, engagement_history::{Covered, History, NAVAL}};
use viewer_foundation::perspective::{Choice, Country, Vision};

struct Sea;

impl Shore for Sea {
    fn province_kind(&self, _province: u32) -> Option<ProvinceKind> {
        Some(ProvinceKind::Sea)
    }

    fn anchor(&self, province: u32, _slot: u32) -> Option<[f32; 3]> {
        Some([province as f32, 0.0, 0.0])
    }
}

fn opaque<T: NativeValue>(value: T) -> Vec<u8> {
    let Field::Opaque(Opaque::Native(value)) = value.field() else { panic!("an opaque value") };
    let mut bytes = Vec::new();
    value.encode_value(&mut bytes).unwrap();
    bytes
}

fn sides(attackers: &[u32], defenders: &[u32]) -> Vec<u8> {
    let mut sides: [NavalCombatSideDetails; 2] = Default::default();
    sides[0].summary.countries = attackers.to_vec();
    sides[1].summary.countries = defenders.to_vec();
    opaque(sides)
}

const KINDS: u32 = 0;

const READ: [TableId; 17] = [
    TableId::NavalCombats,
    TableId::NavalProvinceCombats,
    TableId::NavalCombatDetails,
    TableId::NavalCombatTransfers,
    TableId::NavalTransferUnits,
    TableId::NavalCombatShips,
    TableId::Ships,
    TableId::TaskForces,
    TableId::TaskForceCounterInputs,
    TableId::Divisions,
    TableId::RailwayGunCounterInputs,
    TableId::NavalResults,
    TableId::NavalResultMembers,
    TableId::NavalResultCountries,
    TableId::CountryLinks,
    TableId::LandCombats,
    TableId::NavalCombatDetails,
];

fn combat(store: &mut Store, slot: u32, id: u64, province: u32) {
    put(store, TableId::NavalCombats, slot, Some(u128::from(id)), &[("province", Cell::U32(province)), ("registry_ordinal", Cell::U32(slot))]);
    put(store, TableId::NavalCombatDetails, slot, Some(u128::from(id)), &[("sides", Cell::Opaque(&sides(&[10], &[11])))]);
    put(store, TableId::NavalProvinceCombats, slot, Some(u128::from(province)), &[("combats", Cell::U64List(&[id])), ("kinds", Cell::Bytes(KINDS))]);
}

fn world(store: &mut Store) {
    for (slot, key) in [(0, 10u128), (1, 11)] {
        put(store, TableId::CountryLinks, slot, Some(key), &[("canonical_slot", Cell::U32(slot))]);
    }
    combat(store, 0, 500, 9);
    put(store, TableId::Divisions, 0, Some(7), &[("province", Cell::U32(3))]);
    put(store, TableId::Divisions, 1, Some(8), &[("province", Cell::U32(4))]);
    put(store, TableId::NavalCombatTransfers, 0, Some(500), &[("transfers", Cell::U64List(&[70]))]);
    put(store, TableId::NavalTransferUnits, 0, Some(70), &[("units", Cell::U64List(&[7]))]);
}

fn campaign(name: &str, status: impl Fn(u32, TableId) -> Status, mut hour: impl FnMut(u32, &mut Store)) -> viewer_layer_core::playhead_host::units::Units {
    units(
        record_runs(&[(0, 400)], &[&[NavalProvinceCombatRow::NAVAL]], |at, store| {
            let at = at as u32;
            for table in READ.into_iter().chain([TableId::LandCombatSides]) {
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

fn history() -> Arc<History> {
    let history = Arc::new(History::default());
    let mut covered = Covered::new(0);
    covered.last = 399;
    covered.viewers.push((0, 0));
    for (combat, flag) in [(500, Engaged::Yes), (501, Engaged::No)] {
        covered.fold(false).record(NAVAL, 0, &[((combat, 0), flag), ((combat, 1), flag)], &[]);
    }
    history.branches.lock().unwrap().insert(0, covered);
    history
}

fn markers(history: &Arc<History>) -> NavalMarkers {
    let layer = NavalMarkers::new(Arc::new(Sea), Some(Index { history: Arc::clone(history), branch: 0 }));
    layer.set_vision(Choice { country: Country::Recorded, fog: false }, Arc::new(Vision::sharing(0, []).without_fog()));
    layer
}

fn side_for(units: viewer_layer_core::playhead_host::units::Units) -> MapSide {
    let history = history();
    let entry = open(units, quick()).unwrap();
    let engagements = Engagements::new(Some(Index { history: Arc::clone(&history), branch: 0 }), Some(0));
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(engagements), Box::new(markers(&history))], View { fog: false, ..View::default() }).unwrap();
    assert_eq!(side.refusals().count(), 0, "{:?}", side.refusals().collect::<Vec<_>>());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side
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

fn layer(side: &MapSide) -> &NavalMarkers {
    side.parts().unwrap().1[1].any().downcast_ref::<NavalMarkers>().unwrap()
}

fn values(side: &MapSide) -> NavalMarkerValues {
    layer(side).values().cloned().expect("the naval markers draw")
}

fn settled_equals_composed(side: &mut MapSide) {
    side.settle_all().unwrap();
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(side).values(), fresh.values());
    assert_eq!(layer(side).refused(), fresh.refused());
}

fn icon_combats(values: &NavalMarkerValues) -> Vec<Result<u64, u64>> {
    values.combats.as_ref().map_or(Vec::new(), |combats| {
        combats.icons.iter().filter(|icon| icon.as_ref().map_or_else(|omitted| omitted.rotates, |icon| icon.rotates)).map(|icon| icon.as_ref().map(|icon| icon.combat).map_err(|omitted| omitted.combat)).collect()
    })
}

#[test]
fn an_input_changed_in_a_step_remakes_the_part_once_and_it_equals_composed() {
    let units = campaign("naval markers once", |_, _| Status::Complete, |at, store| {
        if at == 200 {
            put(store, TableId::NavalCombatDetails, 0, None, &[("sides", Cell::Opaque(&sides(&[10], &[])))]);
            put(store, TableId::NavalCombats, 0, None, &[("registry_ordinal", Cell::U32(3))]);
        }
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    assert_eq!(icon_combats(&values(&side)), [Ok(500)], "the engaged combat's icon in its own province");
    let made = layer(&side).made();
    reach(&mut side, 204);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).made() - made, 1, "two tables changed in one step remake it once");
    settled_equals_composed(&mut side);
    let changed = values(&side);
    assert!(changed.gated.contains(&500), "a side with no countries erases the icon");
    assert!(icon_combats(&changed).is_empty());
}

#[test]
fn a_part_changed_twice_before_it_settles_shows_its_newest_values() {
    let units = campaign("naval markers twice", |_, _| Status::Complete, |at, store| match at {
        200 => put(store, TableId::NavalCombatDetails, 0, None, &[("sides", Cell::Opaque(&sides(&[10], &[])))]),
        202 => put(store, TableId::NavalCombatDetails, 0, None, &[("sides", Cell::Opaque(&sides(&[10], &[11, 12])))]),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let settles = side.hold_settles();
    reach(&mut side, 204);
    viewer_layer_core::test_support::recorded::released_unit_by_unit(&mut side, settles);
    settled_equals_composed(&mut side);
    assert_eq!(icon_combats(&values(&side)), [Ok(500)], "the newest sides admit it again");
}

#[test]
fn a_combat_deleted_and_another_inserted_in_its_slot_updates_the_part() {
    let units = campaign("naval markers slots", |_, _| Status::Complete, |at, store| match at {
        200 => {
            for table in [TableId::NavalCombats, TableId::NavalCombatDetails, TableId::NavalProvinceCombats, TableId::NavalCombatTransfers] {
                store.table(table.index()).unwrap().delete(0).unwrap();
            }
        }
        201 => combat(store, 0, 501, 9),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 200);
    settled_equals_composed(&mut side);
    assert!(values(&side).combats.is_none(), "no combat to place");
    reach(&mut side, 201);
    settled_equals_composed(&mut side);
    let placed = values(&side);
    assert_eq!(icon_combats(&placed), [Ok(501)]);
    assert!(placed.live.contains(&501) && !placed.live.contains(&500));
}

#[test]
fn tables_not_wholly_recorded_draw_as_today_draws_them() {
    let units = campaign(
        "naval markers partial",
        |at, table| match (at, table) {
            (240..=247, TableId::NavalCombatShips) => Status::Truncated { missing: 1 },
            (300..=307, TableId::CountryLinks) => Status::NotRecorded,
            _ => Status::Complete,
        },
        |_, _| {},
    );
    let mut side = side_for(units);
    reach(&mut side, 243);
    settled_equals_composed(&mut side);
    let partial = values(&side);
    let combats = partial.combats.as_ref().unwrap();
    assert!(combats.hq.as_ref().is_err_and(|cause| cause.contains("NavalCombatShips")), "{:?}", combats.hq);
    let home = combats.icons.iter().find(|icon| icon.as_ref().map_or_else(|omitted| omitted.rotates, |icon| icon.rotates));
    assert!(home.is_some_and(|icon| icon.as_ref().is_err_and(|omitted| omitted.cause.contains("NavalCombatShips"))), "an engaged icon's variant reads the census: {:?}", combats.icons);
    reach(&mut side, 303);
    side.settle_all().unwrap();
    assert!(layer(&side).refused().is_some_and(|why| why.contains("country link")), "{:?}", layer(&side).refused());
    assert!(layer(&side).values().is_none(), "a refused layer keeps nothing");
    reach(&mut side, 320);
    settled_equals_composed(&mut side);
    assert_eq!(icon_combats(&values(&side)), [Ok(500)]);
}

#[test]
fn a_view_that_hides_the_layer_declares_nothing_and_shown_again_composes() {
    let units = campaign("naval markers hidden", |_, _| Status::Complete, |at, store| {
        if at == 200 {
            put(store, TableId::NavalCombatDetails, 0, None, &[("sides", Cell::Opaque(&sides(&[10], &[])))]);
            put(store, TableId::NavalCombats, 0, None, &[("registry_ordinal", Cell::U32(3))]);
        }
    });
    let mut hidden = View { fog: false, ..View::default() };
    hidden.overlay.naval_markers = false;
    let kinds = replay_recording::schema::schema().unwrap();
    let mut inputs = viewer_layer_core::layers::Inputs::new(&kinds, "naval_markers");
    markers(&history()).inputs(&hidden, &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "the naval markers switched off declare nothing");
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.set_view(hidden).unwrap();
    hold(&mut side, 206);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).carried().entities, 0, "a view that draws nothing carries nothing, though steps changed its tables");
    assert_eq!(layer(&side).held(), markers(&history()).held(), "and holds what an empty layer holds");
    assert!(layer(&side).values().is_none(), "nothing is kept while hidden");
    settled_equals_composed(&mut side);
    side.set_view(View { fog: false, ..View::default() }).unwrap();
    frames_until(&mut side, |side, frame| frame.steps > 0 && !side.composing() && side.parts().is_some_and(|(_, layers)| layers[1].any().downcast_ref::<NavalMarkers>().is_some_and(|layer| layer.refused().is_none())));
    settled_equals_composed(&mut side);
    let shown = values(&side);
    assert!(shown.gated.contains(&500), "composed again, it shows the side with no countries written at hour 200 while hidden");
    assert!(icon_combats(&shown).is_empty(), "which erases the icon");
}

