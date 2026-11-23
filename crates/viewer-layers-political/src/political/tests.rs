
use super::{Content, Installer, LEFT_OUT, Ordering, Political, Row, Set, Whole};
use content_reader_map::map::{MapData, ProvinceDefinition, ProvinceKind};
use std::{collections::HashSet, sync::Arc};
use viewer_borders::{
    borders::{BorderClass, BorderDraw, BorderMesh, BorderMeshes, Vertex},
    demilitarized::{Zone, ZonePlan},
};
use map_display_data::landscape::Regions;

const PROVINCES: u32 = 40;

fn content() -> Arc<Content> {
    let (map, meshes) = world();
    Arc::new(Content::new(Arc::new(map), Arc::new(meshes)))
}

fn world() -> (MapData, BorderMeshes) {
    let definition = |id| (id, ProvinceDefinition { id, rgb: [0; 3], kind: ProvinceKind::Land, continent: 0 });
    let map = MapData { width: PROVINCES, height: 1, province_ids: (1..=PROVINCES).collect(), definitions: (1..=PROVINCES).map(definition).collect(), country_colors: Default::default() };
    let vertex = Vertex { position: [0.0; 3], uv: [0; 2] };
    let draws: Vec<BorderDraw> = (1..PROVINCES).map(|a| BorderDraw { province_a: a, province_b: a + 1, chunk: (a - 1) / 10, base_vertex: (a - 1) % 10 * 4, vertex_count: 4, index_count: 6, coarse_base_vertex: (a - 1) % 10 * 4, coarse_vertex_count: 4 }).collect();
    let chunks: Vec<Vec<Vertex>> = (0..PROVINCES.div_ceil(10)).map(|_| vec![vertex; 40]).collect();
    let mesh = |class| BorderMesh { class, chunks: if class == BorderClass::Province { chunks.clone() } else { Vec::new() }, coarse_chunks: if class == BorderClass::Province { chunks.clone() } else { Vec::new() }, draws: if class == BorderClass::Province { draws.clone() } else { Vec::new() } };
    let meshes = BorderMeshes { meshes: BorderClass::ALL.iter().map(|&class| mesh(class)).collect(), hidden: BorderMesh { class: BorderClass::Province, chunks: Vec::new(), coarse_chunks: Vec::new(), draws: Vec::new() }, indices: vec![0, 2, 1, 2, 3, 1], one_pixel_provinces: vec![] };
    (map, meshes)
}

#[test]
fn a_zone_change_reclassifies_the_moved_states_edges_and_patches_their_chunks() {
    let mut layer = Political::new(content());
    let edges = layer.content.edges.len();
    assert_eq!(edges, PROVINCES as usize - 1);
    layer.state_of = (0..=PROVINCES).map(|province| if (21..=23).contains(&province) { 20 } else { 10 }).collect();
    layer.owner_of = vec![1; PROVINCES as usize + 1];
    layer.class = vec![LEFT_OUT; edges];
    (0..edges as u32).for_each(|edge| layer.classify(edge));
    (layer.edges_changed, layer.zone_plan_stale) = (false, false);
    layer.pending_chunks.clear();
    assert!(layer.class.iter().all(|&class| class != LEFT_OUT), "no zone, no boundary left out");

    layer.rezone(HashSet::from([20]));
    assert_eq!(layer.edges.len(), 4, "the boundaries of provinces 21 to 23 alone, of {edges}");
    assert!(layer.zone_plan_stale && !layer.edges_changed);
    let mut marked = std::mem::take(&mut layer.edges);
    let _ = marked.settle(|edge| {
        layer.classify(edge);
        Ok(())
    });
    let left: Vec<usize> = (0..edges).filter(|&edge| layer.class[edge] == LEFT_OUT).collect();
    assert_eq!(left, [19, 22], "the boundaries 20-21 and 23-24 are split by the zone");
    assert!(!layer.edges_changed, "split boundaries in the land chunks patch the plan, not rebuild it");
    assert_eq!(layer.pending_chunks.take(), [1, 2], "the chunks holding them");

    let (map, meshes) = world();
    let field = |values: &[u32]| map_display_data::province_field::ProvinceField(Arc::new(map_display_data::province_field::Storage::Dense(Arc::new(values.to_vec()))));
    let regions = Regions { country_of: field(&layer.owner_of), owner_of: field(&layer.owner_of), state_of: field(&layer.state_of), country_colors: Default::default(), warnings: Vec::new(), strategic_attributes: Default::default(), mode: None };
    let whole = ZonePlan::new(&meshes, &regions, &Zone::new(&map, &regions, &std::collections::BTreeSet::from([20])));
    assert_eq!(layer.zone_plan_of_its_borders(&meshes), whole);
    assert_ne!(whole, ZonePlan::default(), "the two split boundaries are drawn by the zone");
}

fn handover() -> (Arc<super::Handover>, Installer) {
    let taken = Arc::new(super::Handover::default());
    let live = Arc::new(std::sync::atomic::AtomicBool::new(true));
    (Arc::clone(&taken), Installer::new(taken, Arc::default(), Arc::default(), live, Arc::new(std::sync::atomic::AtomicU64::new(1))))
}

fn set(hour: i32, composed: u64, province: u32, key: u32) -> Set {
    Set { hour, from: hour, composed, whole: None, provinces: vec![(province, key)], keys: Vec::new(), naval: Vec::new(), plans: None, built_ns: 0, digest: None }
}

fn whole(hour: i32, composed: u64, keys_of: Vec<u32>) -> Set {
    let field = |values: &[u32]| map_display_data::province_field::ProvinceField(Arc::new(map_display_data::province_field::Storage::Dense(Arc::new(values.to_vec()))));
    let regions = Regions { country_of: field(&keys_of), owner_of: field(&keys_of), state_of: field(&keys_of), country_colors: Default::default(), warnings: Vec::new(), strategic_attributes: Default::default(), mode: None };
    let whole = Whole { regions: Arc::new(regions), keys_of, colours: Vec::new(), naval: std::collections::BTreeMap::new() };
    Set { hour, from: hour, composed, whole: Some(whole), provinces: Vec::new(), keys: Vec::new(), naval: Vec::new(), plans: None, built_ns: 0, digest: None }
}

#[test]
fn a_frame_writes_the_hours_its_head_has_reached_and_leaves_the_rest() {
    let (sets, mut installer) = handover();
    for hour in [10, 22, 34, 46] {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    let picked = installer.pick(Some(25));
    assert_eq!(picked.hour, Some(22), "the newest hour this head has reached");
    assert_eq!(picked.provinces, [10, 22], "and nothing of hour 34");
    assert_eq!(installer.pick(Some(25)).hour, None, "the same head writes nothing twice");
    assert_eq!(installer.pick(Some(40)).provinces, [34]);
    assert_eq!(installer.pick(Some(60)).provinces, [46]);
    assert!(installer.pick(Some(72)).hour.is_none(), "nothing is left");
}

#[test]
fn a_pause_writes_every_hour_the_queue_holds() {
    let (sets, mut installer) = handover();
    for hour in [10, 22, 34, 46] {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    let picked = installer.pick(None);
    assert_eq!(picked.hour, Some(46));
    assert_eq!(picked.provinces, [10, 22, 34, 46]);
    assert!(!installer.owed, "nothing is left unwritten");
}

#[test]
fn a_seek_writes_nothing_of_the_hours_it_left() {
    let (sets, mut installer) = handover();
    for hour in [10, 22, 34, 46] {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    installer.generation.store(2, Ordering::Release);
    let picked = installer.pick(Some(60));
    assert_eq!(picked.hour, None, "no hour of the picture the seek left");
    assert!(picked.provinces.is_empty());
    sets.push(whole(900, 2, vec![0, 7]));
    let picked = installer.pick(Some(900));
    assert_eq!(picked.hour, Some(900));
    assert!(picked.whole.is_some(), "the seek's own picture");
    assert_eq!(installer.keys_of, [0, 7], "and what is on the GPU is that picture's");
}

#[test]
fn a_speed_change_leaves_the_dated_hours_standing() {
    let (sets, mut installer) = handover();
    for hour in [10, 22, 34, 46] {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    assert_eq!(installer.pick(Some(12)).provinces, [10]);
    for head in [13, 14, 15] {
        assert!(installer.pick(Some(head)).hour.is_none(), "a slower head reaches no new hour");
    }
    assert!(installer.due(Some(15)).is_none(), "and none is owed at that head");
    assert_eq!(installer.pick(Some(46)).provinces, [22, 34, 46], "and the rest when it gets there");
}

#[test]
fn a_head_that_went_backwards_writes_nothing_of_the_hours_it_has_left() {
    let (sets, mut installer) = handover();
    for hour in [10, 22, 34, 46] {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    assert_eq!(installer.pick(Some(46)).hour, Some(46));
    assert!(installer.pick(Some(20)).hour.is_none(), "nothing is written for a head that went back");
    assert!(installer.due(Some(20)).is_none(), "and nothing of those hours is owed it, so the picture stands where it was");
    installer.generation.store(2, Ordering::Release);
    sets.push(whole(20, 2, vec![0, 3]));
    let picked = installer.pick(Some(20));
    assert_eq!(picked.hour, Some(20));
    assert!(picked.whole.is_some());
}

#[test]
fn a_fill_that_refused_a_changed_write_waits_for_the_whole_one() {
    let (sets, mut installer) = handover();
    installer.refused = true;
    sets.push(set(22, 1, 22, 22));
    let picked = installer.pick(Some(30));
    assert_eq!(picked.hour, None);
    assert!(picked.provinces.is_empty());
    sets.push(whole(34, 1, vec![0, 1]));
    let picked = installer.pick(Some(40));
    assert_eq!(picked.hour, Some(34), "the whole fill voids the set it refused");
    assert!(!installer.refused);
}

#[test]
fn a_whole_fill_ahead_of_the_head_waits_for_its_hour() {
    let (sets, mut installer) = handover();
    sets.push(set(102, 1, 2, 2));
    sets.push(set(104, 1, 4, 4));
    sets.push(whole(148, 1, vec![0, 9]));
    let picked = installer.pick(Some(103));
    assert!(picked.whole.is_none(), "the whole fill of hour 148 is not this head's");
    assert_eq!((picked.hour, picked.provinces), (Some(102), vec![2]));
    assert_eq!(installer.pick(Some(147)).provinces, [4]);
    let picked = installer.pick(Some(148));
    assert_eq!(picked.hour, Some(148));
    assert!(picked.whole.is_some());
    assert_eq!(installer.keys_of, [0, 9]);
}

#[test]
fn a_head_inside_a_long_run_of_changes_writes_the_hours_it_reached() {
    let (sets, mut installer) = handover();
    for hour in 1..=1440 {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    let picked = installer.pick(Some(700));
    assert_eq!(picked.hour, Some(700));
    assert_eq!(picked.provinces, (1..=700).collect::<Vec<usize>>());
}

#[test]
fn a_hand_over_past_what_it_keeps_apart_folds_its_oldest_hours_instead_of_losing_them() {
    let (sets, mut installer) = handover();
    let hours = super::SETS_HELD as i32 + 40;
    for hour in 1..=hours {
        sets.push(set(hour, 1, hour as u32, hour as u32));
    }
    let picked = installer.pick(Some(40));
    assert!(picked.hour.is_none(), "the folded hours 1 to 41 are not shown before hour 41");
    let picked = installer.pick(None);
    assert_eq!(picked.hour, Some(hours), "the newest hour dated");
    assert_eq!(picked.provinces.len(), hours as usize, "and every province of those hours");
    assert_eq!(installer.keys_of[hours as usize], hours as u32, "with the newest hour's key");
    let (sets, _installer) = handover();
    for hour in 1..=hours {
        sets.push(set(hour, 1, 7, hour as u32));
    }
    let held = sets.0.lock().unwrap();
    assert_eq!(held.front().map(|set| set.provinces.clone()), Some(vec![(7, 41)]), "the folded hours 1 to 41, as hour 41 left province 7");
}

#[test]
fn the_hours_of_a_publication_are_dated_one_change_set_an_hour() {
    let mut layer = Political::new(content());
    layer.links = [(7, 0), (8, 1)].into_iter().collect();
    layer.natives = [(0, 70), (1, 80)].into_iter().collect();
    layer.resize();
    layer.provinces.clear();
    let row = |owner, controller| Row { state: Some(5), owner: Some(owner), controller: Some(controller) };
    for id in 1..=3 {
        layer.paint_from(id, Some(row(7, 7)));
    }
    assert_eq!(layer.country_of[1..=3], [1, 1, 1], "every province is country link 7's, whose slot is 0");
    layer.pending_provinces.clear();

    layer.hour_rows = vec![(100, vec![(1, row(7, 8))]), (104, vec![(2, row(8, 8))]), (108, vec![(1, row(8, 8)), (3, row(7, 8))])];
    layer.paint_hours();
    assert_eq!(layer.hour_sets, vec![(100, vec![(1, 2)]), (104, vec![(2, 2)]), (108, vec![(1, 2), (3, 2)])]);
    assert!(layer.hour_rows.is_empty() && layer.pending_provinces.is_empty(), "every hour is dated and nothing is left for the publication's own");
    assert_eq!(layer.interior_keys, [2].into(), "the key the hours painted with is coloured whatever holds it at the end");
    assert_eq!(layer.country_of[1..=3], [2, 2, 2], "and the arrays stand where the publication's last hour left them");
    assert_eq!(layer.recorded[2], Some(row(8, 8)), "the cells each province was painted from are kept for the hours that write only some of them");
}

#[test]
fn a_colour_changed_inside_a_publication_shows_from_the_hour_it_changed() {
    use std::collections::BTreeMap;
    let mut layer = Political::new(content());
    let (old, new) = ([10, 20, 30], [255, 0, 0]);
    layer.colours = vec![None, Some(new)];
    layer.colour_native = vec![super::NONE, 70];
    let channel = |value: f32| Some(value.to_bits());
    let appearance = vec![(104, vec![(70, 0, channel(1.0)), (70, 1, channel(0.0)), (70, 2, channel(0.0))])];
    let painted = || vec![(102, vec![(3, 1)]), (106, vec![(4, 1)])];
    let dated = layer.dated_hours(painted(), &BTreeMap::from([(1, (Some(old), 70))]), &appearance, None, 110).0;
    assert_eq!(dated, vec![(102, vec![(3, 1)], vec![(1, Some(old))]), (104, vec![], vec![(1, Some(new))]), (106, vec![(4, 1)], vec![(1, Some(new))])]);
    let dated = layer.dated_hours(painted(), &BTreeMap::from([(1, (Some(old), 80))]), &[], Some(105), 110).0;
    assert_eq!(dated, vec![(102, vec![(3, 1)], vec![(1, Some(old))]), (105, vec![], vec![(1, Some(new))]), (106, vec![(4, 1)], vec![(1, Some(new))])]);
    let overlord = [0, 0, 255];
    layer.colours = vec![None, Some(new), Some(new)];
    layer.colour_native = vec![super::NONE, 70, 70];
    let later = vec![(135, vec![(70, 0, channel(1.0)), (70, 2, channel(0.0))])];
    let before = BTreeMap::from([(1, (Some(old), 80)), (2, (Some(overlord), 70))]);
    let dated = layer.dated_hours(vec![(102, vec![(3, 1)])], &before, &later, Some(105), 148).0;
    assert_eq!(dated, vec![(102, vec![(3, 1)], vec![(1, Some(old))]), (105, vec![], vec![(1, Some(overlord))]), (135, vec![], vec![(1, Some(new)), (2, Some(new))])]);
    layer.colours = vec![None, None, Some(overlord)];
    let dated = layer.dated_hours(vec![(105, vec![(5, 2)]), (120, vec![(5, 1)])], &BTreeMap::from([(2, (None, super::NONE))]), &[], None, 148).0;
    assert_eq!(dated[0], (105, vec![(5, 2)], vec![(2, Some(overlord))]));
}

#[test]
fn borders_and_zones_are_dated_at_the_hours_they_changed() {
    let mut layer = Political::new(content());
    layer.links = [(7, 0), (8, 1)].into_iter().collect();
    layer.natives = [(0, 70), (1, 80)].into_iter().collect();
    layer.resize();
    let row = |owner, state| Row { state: Some(state), owner: Some(owner), controller: Some(owner) };
    for id in 1..=PROVINCES {
        layer.paint_from(id, Some(row(7, if id >= 30 { 9 } else { 5 })));
    }
    let edges = layer.content.edges.len();
    layer.class = vec![LEFT_OUT; edges];
    (0..edges as u32).for_each(|edge| layer.classify(edge));
    layer.prepare();
    assert!(layer.classes.is_some() && !layer.edges_changed, "the plans stand before the publication");
    (layer.edges, layer.pending_chunks, layer.hour_plans) = (Default::default(), Default::default(), Vec::new());
    layer.pending_provinces.clear();

    layer.hour_rows = vec![(104, vec![(2, row(8, 5))])];
    layer.pending_zone = Some((140, HashSet::from([9])));
    layer.hour = 148;
    layer.paint_hours();
    let hours: Vec<replay_data::Hour> = layer.hour_plans.iter().map(|(at, _)| *at).collect();
    assert_eq!(hours, [104, 140], "the transfer's plans at 104, the zone's at 140");
    assert_eq!(layer.class[0], BorderClass::Country.bucket() as u8, "the boundary 1-2 is a country border");
    assert_ne!(layer.hour_plans[0].1.1, layer.hour_plans[1].1.1, "the zone's plan changes at 140, not at 104");
    assert_eq!(layer.hour_plans[0].1.1, ZonePlan::default(), "and no zone stands at 104");
}

#[test]
fn a_whole_folded_with_a_later_write_keeps_the_write() {
    let (_sets, mut installer) = handover();
    let mut front = whole(100, 1, vec![0, 4, 4]);
    front.fold(set(101, 1, 2, 9));
    installer.sets.push_back(front);
    let picked = installer.pick(Some(101));
    assert!(picked.whole.is_some());
    assert_eq!(installer.keys_of[2], 9, "province 2 took key 9 at 101, after the whole fill of 100");
    assert_eq!(picked.provinces, [2]);
}

#[test]
fn a_head_held_back_holds_a_bounded_queue() {
    let (sets, mut installer) = handover();
    for hour in 1..=(super::SETS_HELD as i32 * 2) {
        sets.push(set(100 + hour, 1, 7, hour as u32));
        installer.pick(Some(100));
    }
    assert!(installer.sets.len() <= super::SETS_HELD, "{} sets held", installer.sets.len());
    let picked = installer.pick(None);
    assert_eq!(installer.keys_of[7], super::SETS_HELD as u32 * 2, "and every change is written once the head moves");
    assert_eq!(picked.provinces, [7]);
}

#[test]
fn an_inner_hour_refusing_a_province_is_not_dated() {
    let mut layer = Political::new(content());
    layer.links = [(7, 0)].into_iter().collect();
    layer.natives = [(0, 70)].into_iter().collect();
    layer.resize();
    let row = |controller: Option<u32>| Row { state: Some(5), owner: Some(7), controller };
    layer.paint_from(3, Some(row(Some(7))));
    layer.pending_provinces.clear();
    layer.hour_rows = vec![(105, vec![(3, row(None))]), (120, vec![(3, row(Some(7)))])];
    layer.paint_hours();
    assert_eq!(layer.undated_from, Some(105), "the hours from 105 on wait for the publication's own");
}

#[test]
fn a_colour_that_cannot_be_followed_is_not_dated() {
    use std::collections::BTreeMap;
    let mut layer = Political::new(content());
    layer.colours = vec![None, Some([255, 0, 0])];
    layer.colour_native = vec![super::NONE, 70];
    let channel = |value: f32| Some(value.to_bits());
    let red = vec![(105, vec![(70, 0, channel(1.0))])];
    let (_, undated) = layer.colour_hours(&BTreeMap::from([(1, (Some(super::UNAVAILABLE_COLOR), 70))]), &red, None);
    assert_eq!(undated, Some(105), "a grey start has no channels to follow");
    let earlier = vec![(103, vec![(80, 0, channel(1.0))])];
    let (_, undated) = layer.colour_hours(&BTreeMap::from([(1, (Some([0, 0, 255]), 80))]), &earlier, Some(105));
    assert_eq!(undated, Some(103), "the former source moved before the change");
}

fn naval_turn(name: &str) -> viewer_layer_core::playhead_host::units::Units {
    use replay_columnar::{manifest::Status, state::Cell, store::Store};
    use replay_data::rows::TableId;
    let column = viewer_layer_core::test_support::recorded::column;
    let mut tables: Vec<TableId> = viewer_foundation::perspective::VISION_TABLES.to_vec();
    tables.extend([TableId::NavalSupremacy, TableId::RelationTuples, TableId::Provinces, TableId::CountryAppearance, TableId::Autonomy, TableId::StateFlags, TableId::States]);
    tables.sort_unstable();
    tables.dedup();
    viewer_layer_core::test_support::recorded::record(name, 24, |at, store| {
        let write = |store: &mut Store, table: TableId, slot: u32, field: &str, cell: Cell<'_>| store.table(table.index()).unwrap().write(slot, column(table, field), cell).unwrap();
        let insert = |store: &mut Store, table: TableId, slot: u32, key: u128| store.table(table.index()).unwrap().insert_at(slot, key).unwrap();
        let supremacy = |store: &mut Store, slot: u32, current: i64, previous: i64| {
            write(store, TableId::NavalSupremacy, slot, "current", Cell::I64(current));
            write(store, TableId::NavalSupremacy, slot, "previous", Cell::I64(previous));
        };
        match at {
            0 => {
                for (slot, raw) in [101u32, 102].into_iter().enumerate() {
                    insert(store, TableId::CountryLinks, slot as u32, u128::from(raw));
                    write(store, TableId::CountryLinks, slot as u32, "canonical_slot", Cell::U32(raw - 100));
                    insert(store, TableId::Countries, slot as u32, u128::from(raw));
                    write(store, TableId::Countries, slot as u32, "canonical_slot", Cell::U32(raw - 100));
                }
                insert(store, TableId::LocalPlayer, 0, 0);
                write(store, TableId::LocalPlayer, 0, "country", Cell::U32(101));
                insert(store, TableId::WarRelations, 0, 101 << 32 | 102);
                for (slot, country) in [101u128, 102].into_iter().enumerate() {
                    insert(store, TableId::NavalSupremacy, slot as u32, 10 << 32 | country);
                }
                supremacy(store, 0, 900_000, 900_000);
                supremacy(store, 1, 100_000, 50_000);
            }
            9 => supremacy(store, 1, 2_000_000, 100_000),
            _ => {}
        }
        for &table in &tables {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
    })
}

fn naval_dated_with_its_hour(name: &str, whole: bool) -> Vec<replay_data::Hour> {
    use viewer_layer_core::{layers::{Layer, View}, vision, playhead_host::{MapSide, open}};
    use crate::naval::Naval;
    use std::collections::{BTreeMap, BTreeSet};
    use viewer_foundation::visibility::{FogContent, Topology, test_defines};
    let definition = |id, kind| (id, ProvinceDefinition { id, rgb: [0; 3], kind, continent: 0 });
    let map = MapData { width: 2, height: 1, province_ids: vec![1, 2], definitions: [definition(1, ProvinceKind::Land), definition(2, ProvinceKind::Sea)].into_iter().collect(), country_colors: Default::default() };
    let empty = |class| BorderMesh { class, chunks: vec![], coarse_chunks: vec![], draws: vec![] };
    let meshes = BorderMeshes { meshes: BorderClass::ALL.iter().map(|&class| empty(class)).collect(), hidden: empty(BorderClass::Province), indices: vec![], one_pixel_provinces: vec![] };
    let fog = FogContent { topology: Topology { land: BTreeSet::from([1]), sea: BTreeSet::from([2]), neighbors: BTreeMap::from([(1, vec![2]), (2, vec![1])]) }, defines: test_defines() };
    let layers: Vec<Box<dyn Layer>> = vec![
        Box::new(Political::new(Arc::new(Content::new(Arc::new(map), Arc::new(meshes))))),
        Box::new(vision::Vision::new(Arc::new(vision::Content::new(Ok(fog))))),
        Box::new(Naval::new(Arc::new(viewer_map_modes::naval_dominance::Content::for_tests(&[(10, true, 100_000)], 60_000)))),
    ];
    let settings = playhead::Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..playhead::Settings::default() };
    let entry = open(naval_turn(name), settings).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    assert!(side.refusals().next().is_none());
    drop(side.take_installs());
    let mut installer = {
        let political = side.parts().unwrap().1.iter().find_map(|layer| layer.any().downcast_ref::<Political>()).unwrap();
        Installer::new(Arc::clone(political.sets.as_ref().expect("the installer was taken")), Arc::default(), Arc::default(), Arc::clone(&political.live), Arc::clone(&political.generation))
    };
    let _held = whole.then(|| side.hold_settles());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    let (mut applied, mut before, mut turned) = (None::<replay_data::Hour>, None, Vec::new());
    for _ in 0..50_000_000u64 {
        let head = match applied {
            None => 0,
            Some(hour) if side.composing() => hour,
            Some(hour) => entry.playhead().newest().unwrap_or(hour).min(hour + 4),
        };
        let frame = side.frame(head).unwrap();
        applied = frame.applied.or(applied);
        let Some(hour) = applied.filter(|_| (frame.steps > 0 || frame.swapped.is_some()) && !side.composing()) else {
            std::thread::yield_now();
            continue;
        };
        if whole {
            side.settle_all().unwrap();
        }
        if side.draws("political") && side.draws("naval") {
            installer.pick(Some(hour));
            let naval = side.parts().unwrap().1.iter().find_map(|layer| layer.any().downcast_ref::<Naval>()).unwrap();
            let attributes = (**naval.attributes()).clone();
            assert_eq!(installer.naval, attributes, "the political sets dated through hour {hour} carry the naval attributes of hour {hour}");
            if before.as_ref().is_some_and(|before| *before != attributes) {
                turned.push(hour);
            }
            before = Some(attributes);
        }
        if hour >= 20 {
            return turned;
        }
    }
    panic!("hour 20 was not reached: applied {applied:?}, composing {}", side.composing());
}

#[test]
fn a_step_dates_naval_attributes_at_their_own_hour() {
    let turned = naval_dated_with_its_hour("political naval step", false);
    assert!(turned.iter().any(|&hour| (9..=16).contains(&hour)), "region 10 turned at hour 9, seen at {turned:?}");
}

#[test]
fn a_whole_settle_dates_naval_attributes_at_their_own_hour() {
    let turned = naval_dated_with_its_hour("political naval whole", true);
    assert!(turned.iter().any(|&hour| (9..=16).contains(&hour)), "region 10 turned at hour 9, seen at {turned:?}");
}

#[derive(Default)]
struct Refusing {
    plans: usize,
}

impl super::Sink for Refusing {
    fn set_political(&mut self, _regions: &Arc<Regions>) -> std::io::Result<()> {
        Ok(())
    }

    fn update_naval(&mut self, _attributes: &dyn Fn(u32) -> super::Attributes, _regions: &[u32]) -> std::io::Result<bool> {
        Ok(true)
    }

    fn update_political(&mut self, _keys_of: &[u32], _colour: &dyn Fn(u32) -> [u8; 3], _provinces: &[usize], _keys: &[u32]) -> std::io::Result<bool> {
        Ok(false)
    }

    fn set_border_plans(&mut self, _classes: &viewer_borders::border_pass::ClassPlan, _zone: &ZonePlan) -> std::io::Result<()> {
        self.plans += 1;
        Ok(())
    }
}

#[test]
fn a_refused_fill_write_keeps_its_hour_and_its_borders() {
    let (sets, mut installer) = handover();
    sets.push(whole(100, 1, vec![0, 4, 4]));
    let mut moved = set(101, 1, 2, 9);
    moved.plans = Some((Default::default(), ZonePlan::default()));
    sets.push(moved);
    let mut scene = Refusing::default();
    let mut phases = [0; 4];
    let picked = installer.pick(Some(100));
    installer.write(&mut scene, picked, Some(100), &mut phases).unwrap();
    assert_eq!(installer.at, Some(100));
    let picked = installer.pick(Some(101));
    installer.write(&mut scene, picked, Some(101), &mut phases).unwrap();
    assert_eq!(installer.at, Some(100), "the refused hour is not on screen");
    assert_eq!(scene.plans, 0, "and its borders are not written");
    assert!(installer.refused && installer.asks_whole.load(Ordering::Acquire), "the whole fill is asked for");
}

const STEP: replay_data::Hour = 12;

const NATIONS: [(u32, u32, u32, [f32; 3]); 3] = [(7, 0, 70, [0.2, 0.2, 0.2]), (8, 1, 80, [0.4, 0.4, 0.4]), (9, 2, 90, [0.6, 0.6, 0.6])];

fn shade(channels: [f32; 3]) -> Option<[u8; 3]> {
    Some(channels.map(|value| (value * 255.0) as u8))
}

fn write_cell(store: &mut replay_columnar::store::Store, table: replay_data::rows::TableId, slot: u32, field: &str, cell: replay_columnar::state::Cell<'_>) {
    store.table(table.index()).unwrap().write(slot, viewer_layer_core::test_support::recorded::column(table, field), cell).unwrap();
}

fn insert_row(store: &mut replay_columnar::store::Store, table: replay_data::rows::TableId, slot: u32, key: u128) {
    store.table(table.index()).unwrap().insert_at(slot, key).unwrap();
}

fn delete_row(store: &mut replay_columnar::store::Store, table: replay_data::rows::TableId, slot: u32) {
    store.table(table.index()).unwrap().delete(slot).unwrap();
}

fn hold(store: &mut replay_columnar::store::Store, id: u32, owner: u32, controller: u32) {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    write_cell(store, TableId::Provinces, id - 1, "owner", Cell::U32(owner));
    write_cell(store, TableId::Provinces, id - 1, "controller", Cell::U32(controller));
}

fn paint_appearance(store: &mut replay_columnar::store::Store, slot: u32, channels: [f32; 3]) {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    for (field, value) in ["red", "green", "blue"].into_iter().zip(channels) {
        write_cell(store, TableId::CountryAppearance, slot, field, Cell::U32(value.to_bits()));
    }
}

fn politics(name: &str, provinces_out: std::ops::RangeInclusive<u32>, mut campaign: impl FnMut(u32, &mut replay_columnar::store::Store)) -> viewer_layer_core::playhead_host::units::Units {
    use replay_columnar::{manifest::Status, state::Cell};
    use replay_data::rows::TableId;
    let tables = [TableId::Provinces, TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::Subjects, TableId::Autonomy, TableId::StateFlags, TableId::States];
    viewer_layer_core::test_support::recorded::record_naming(name, 48, &[b"puppet", b"outage"], |at, store| {
        if at == 0 {
            for (slot, &(raw, canonical, native, channels)) in NATIONS.iter().enumerate() {
                let slot = slot as u32;
                insert_row(store, TableId::CountryLinks, slot, u128::from(raw));
                write_cell(store, TableId::CountryLinks, slot, "canonical_slot", Cell::U32(canonical));
                insert_row(store, TableId::Countries, slot, u128::from(native));
                write_cell(store, TableId::Countries, slot, "canonical_slot", Cell::U32(canonical));
                insert_row(store, TableId::CountryAppearance, slot, u128::from(native));
                paint_appearance(store, slot, channels);
            }
            for id in 1..=PROVINCES {
                insert_row(store, TableId::Provinces, id - 1, u128::from(id));
                write_cell(store, TableId::Provinces, id - 1, "state", Cell::U32(if id >= 30 { 9 } else { 5 }));
                hold(store, id, 7, 7);
            }
            for (slot, state) in [5u128, 9].into_iter().enumerate() {
                insert_row(store, TableId::States, slot as u32, state);
            }
            insert_row(store, TableId::StateFlags, 0, 9);
            write_cell(store, TableId::StateFlags, 0, "demilitarized", Cell::U8(1));
        }
        campaign(at, store);
        for table in tables {
            let status = if table == TableId::Provinces && provinces_out.contains(&at) { Status::Unavailable { reason: 1 } } else { Status::Complete };
            store.status(table.index(), status, true, Some(at as i32), true).unwrap();
        }
    })
}

fn political_side(units: viewer_layer_core::playhead_host::units::Units, content: Arc<Content>) -> viewer_layer_core::playhead_host::MapSide {
    use viewer_layer_core::{layers::{Layer, View}, playhead_host::{MapSide, open}};
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(Political::new(content))];
    let speed = f64::from(STEP);
    let settings = playhead::Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 8, speed, rate: 1.0, ..playhead::Settings::default() };
    let entry = open(units, settings).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    assert!(side.refusals().next().is_none());
    drop(side.take_installs());
    entry.playhead().play(speed);
    entry.playhead().seek(0);
    side
}

fn political(side: &viewer_layer_core::playhead_host::MapSide) -> &Political {
    side.parts().unwrap().1.iter().find_map(|layer| layer.any().downcast_ref::<Political>()).unwrap()
}

fn frame_to(side: &mut viewer_layer_core::playhead_host::MapSide, sets: &mut Vec<Set>, target: replay_data::Hour) {
    let handover = Arc::clone(political(side).sets.as_ref().expect("the installer was taken"));
    for _ in 0..50_000_000u64 {
        viewer_layer_core::test_support::recorded::reads_landed(side);
        let built = side.entry().playhead().newest().is_some_and(|newest| newest >= target);
        let ready = built && side.applied().is_some() && !side.composing();
        let head = if ready { target } else { side.applied().unwrap_or(0) };
        let frame = side.frame(head).unwrap();
        sets.extend(handover.take());
        if side.applied().is_some_and(|applied| applied >= target) && !side.composing() {
            assert_eq!(side.applied(), Some(target), "the steps end at the hours a test names: {}", viewer_layer_core::test_support::recorded::seen(side));
            return;
        }
        assert!(!ready || frame.steps == 0, "the frame at {target} took only part of the steps built through it: {}", viewer_layer_core::test_support::recorded::seen(side));
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    panic!("hour {target} was not applied: {}", viewer_layer_core::test_support::recorded::seen(side));
}

#[derive(Default)]
struct Screen {
    refuse: bool,
    blank: bool,
    plans: Option<(viewer_borders::border_pass::ClassPlan, ZonePlan)>,
    plans_written: usize,
}

impl super::Sink for Screen {
    fn set_political(&mut self, regions: &Arc<Regions>) -> std::io::Result<()> {
        self.blank = (1..=PROVINCES as usize).all(|id| regions.country_of.get(id) == Some(&0));
        Ok(())
    }

    fn update_naval(&mut self, _attributes: &dyn Fn(u32) -> super::Attributes, _regions: &[u32]) -> std::io::Result<bool> {
        Ok(!self.refuse)
    }

    fn update_political(&mut self, _keys_of: &[u32], _colour: &dyn Fn(u32) -> [u8; 3], _provinces: &[usize], _keys: &[u32]) -> std::io::Result<bool> {
        Ok(!self.refuse)
    }

    fn set_border_plans(&mut self, classes: &viewer_borders::border_pass::ClassPlan, zone: &ZonePlan) -> std::io::Result<()> {
        self.plans = Some((classes.clone(), zone.clone()));
        self.plans_written += 1;
        Ok(())
    }
}

fn certified(name: &str, units: viewer_layer_core::playhead_host::units::Units, content: Arc<Content>, targets: &[replay_data::Hour], mut truth: impl FnMut(replay_data::Hour, &Installer, &Screen) -> Option<String>) {
    let mut side = political_side(units, content);
    let mut sets = Vec::new();
    for &target in targets {
        frame_to(&mut side, &mut sets, target);
    }
    let generation = Arc::clone(&political(&side).generation);
    let taken = Arc::new(super::Handover::default());
    for set in sets {
        taken.push(set);
    }
    let mut installer = Installer::new(taken, Arc::default(), Arc::default(), Arc::new(std::sync::atomic::AtomicBool::new(true)), generation);
    let (mut screen, mut phases) = (Screen::default(), [0; 4]);
    let mut wrong = Vec::new();
    for hour in 0..=36 {
        let picked = installer.pick(Some(hour));
        installer.write(&mut screen, picked, Some(hour), &mut phases).unwrap();
        if installer.at == Some(hour)
            && let Some(why) = truth(hour, &installer, &screen)
        {
            wrong.push(format!("hour {hour}: {why}"));
        }
    }
    assert!(wrong.is_empty(), "{name}: the installer said these hours were on screen with another hour's picture: {wrong:#?}");
}

#[test]
fn a_core_table_unavailable_inside_publications_certifies_none_of_its_hours() {
    let units = politics("political core outage", 15..=26, |_, _| {});
    certified("core outage", units, content(), &[12, 24, 36], |hour, installer, _| {
        let recorded = !(15..=26).contains(&hour);
        let key = installer.keys_of.get(1).copied().unwrap_or(0);
        (recorded != (key == 1)).then(|| format!("province 1 shows key {key}, the map {} recorded", if recorded { "being" } else { "not" }))
    });
}

#[test]
fn a_demilitarised_row_gone_inside_a_publication_ends_the_zone_at_its_hour() {
    use replay_data::rows::TableId;
    let units = politics("political zone row gone", 1..=0, |at, store| {
        if at == 15 {
            delete_row(store, TableId::StateFlags, 0);
        }
    });
    certified("zone row gone", units, content(), &[12, 24, 36], |hour, _, screen| {
        let zoned = hour < 15;
        let shown = screen.plans.as_ref().is_some_and(|(_, zone)| *zone != ZonePlan::default());
        (zoned != shown).then(|| format!("the borders show {} zone", if shown { "a" } else { "no" }))
    });
}

#[test]
fn a_link_written_inside_a_publication_undates_the_provinces_painted_before_it() {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    let units = politics("political link rewritten", 1..=0, |at, store| match at {
        15 => hold(store, 2, 8, 8),
        19 => write_cell(store, TableId::CountryLinks, 1, "canonical_slot", Cell::U32(2)),
        _ => {}
    });
    certified("link rewritten", units, content(), &[12, 24, 36], |hour, installer, _| {
        let truth = match hour {
            ..15 => 1,
            15..19 => 2,
            _ => 3,
        };
        let key = installer.keys_of.get(2).copied().unwrap_or(0);
        (key != truth).then(|| format!("province 2 shows key {key}, not {truth}"))
    });
}

#[test]
fn a_core_table_unavailable_and_back_inside_one_step_certifies_none_of_its_hours() {
    let units = politics("political core blink", 15..=17, |_, _| {});
    certified("core blink", units, content(), &[12, 24, 36], |hour, installer, _| {
        let recorded = !(15..=17).contains(&hour);
        let key = installer.keys_of.get(1).copied().unwrap_or(0);
        (recorded != (key == 1)).then(|| format!("province 1 shows key {key}, the map {} recorded", if recorded { "being" } else { "not" }))
    });
}

#[test]
fn a_link_written_inside_a_publication_undates_the_provinces_already_on_it() {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    let units = politics("political link under a province", 1..=0, |at, store| match at {
        5 => hold(store, 2, 8, 8),
        19 => write_cell(store, TableId::CountryLinks, 1, "canonical_slot", Cell::U32(2)),
        _ => {}
    });
    certified("link under a province", units, content(), &[12, 24, 36], |hour, installer, _| {
        let truth = match hour {
            ..5 => 1,
            5..19 => 2,
            _ => 3,
        };
        let key = installer.keys_of.get(2).copied().unwrap_or(0);
        (key != truth).then(|| format!("province 2 shows key {key}, not {truth}"))
    });
}

#[test]
fn appearance_rows_moved_between_slots_undate_the_appearance_writes() {
    use replay_data::rows::TableId;
    let red = [1.0, 0.0, 0.0];
    let units = politics("political appearance reslotted", 1..=0, |at, store| match at {
        0 => {
            hold(store, 10, 8, 8);
            hold(store, 20, 9, 9);
        }
        15 => paint_appearance(store, 1, red),
        27 => {
            delete_row(store, TableId::CountryAppearance, 1);
            delete_row(store, TableId::CountryAppearance, 2);
            insert_row(store, TableId::CountryAppearance, 1, 90);
            paint_appearance(store, 1, NATIONS[2].3);
            insert_row(store, TableId::CountryAppearance, 2, 80);
            paint_appearance(store, 2, red);
        }
        _ => {}
    });
    certified("appearance reslotted", units, content(), &[12, 36], |hour, installer, _| {
        let colour = |key: usize| installer.colours.get(key).copied().flatten();
        let eighty = if hour < 15 { shade(NATIONS[1].3) } else { shade(red) };
        let ninety = shade(NATIONS[2].3);
        (colour(2) != eighty || colour(3) != ninety).then(|| format!("keys 2 and 3 are {:?} and {:?}, not {eighty:?} and {ninety:?}", colour(2), colour(3)))
    });
}

#[test]
fn an_inner_keys_colour_refusal_does_not_blank_the_publications_end() {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    let units = politics("political inner colour refusal", 1..=0, |at, store| match at {
        0 => {
            insert_row(store, TableId::CountryLinks, 3, 6);
            write_cell(store, TableId::CountryLinks, 3, "canonical_slot", Cell::U32(5));
            insert_row(store, TableId::Subjects, 0, 6 << 32 | 9);
            write_cell(store, TableId::Subjects, 0, "start_hour", Cell::I32(0));
            write_cell(store, TableId::Subjects, 0, "end_hour", Cell::I32(0));
            write_cell(store, TableId::Subjects, 0, "cancel", Cell::U8(0));
            insert_row(store, TableId::Autonomy, 0, 9);
            write_cell(store, TableId::Autonomy, 0, "state", Cell::Bytes(0));
        }
        15 => hold(store, 3, 7, 9),
        17 => hold(store, 3, 7, 7),
        _ => {}
    });
    let (mut map, meshes) = world();
    map.country_colors.use_overlord_color.insert(b"puppet".to_vec(), true);
    let content = Arc::new(Content::new(Arc::new(map), Arc::new(meshes)));
    certified("inner colour refusal", units, content, &[12, 24, 36], |hour, _, screen| {
        let refused = (15..=16).contains(&hour);
        (screen.blank != refused).then(|| format!("the fill is {}", if screen.blank { "blank" } else { "drawn" }))
    });
}

#[test]
fn a_subject_relation_undone_inside_a_publication_dates_none_of_the_hours_between() {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    let units = politics("political relation undone", 1..=0, |at, store| match at {
        0 => {
            hold(store, 20, 9, 9);
            insert_row(store, TableId::Subjects, 0, 8 << 32 | 9);
            write_cell(store, TableId::Subjects, 0, "start_hour", Cell::I32(0));
            write_cell(store, TableId::Subjects, 0, "end_hour", Cell::I32(0));
            write_cell(store, TableId::Subjects, 0, "cancel", Cell::U8(0));
            insert_row(store, TableId::Autonomy, 0, 9);
            write_cell(store, TableId::Autonomy, 0, "state", Cell::Bytes(0));
        }
        15 => write_cell(store, TableId::Autonomy, 0, "state", Cell::Bytes(1)),
        19 => write_cell(store, TableId::Autonomy, 0, "state", Cell::Bytes(0)),
        _ => {}
    });
    let (mut map, meshes) = world();
    map.country_colors.use_overlord_color.insert(b"puppet".to_vec(), true);
    let content = Arc::new(Content::new(Arc::new(map), Arc::new(meshes)));
    certified("relation undone", units, content, &[12, 24, 36], |hour, installer, _| {
        let colour = installer.colours.get(3).copied().flatten();
        let truth = if (15..19).contains(&hour) { shade(NATIONS[2].3) } else { shade(NATIONS[1].3) };
        (colour != truth).then(|| format!("key 3 is {colour:?}, not {truth:?}"))
    });
}

#[test]
fn an_old_colour_source_written_caps_the_hours_when_the_new_one_is_not_followed() {
    use replay_columnar::state::Cell;
    use replay_data::rows::TableId;
    let (red, blue) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    let units = politics("political old colour source", 1..=0, |at, store| match at {
        0 => {
            hold(store, 10, 8, 8);
            insert_row(store, TableId::CountryAppearance, 3, 85);
            paint_appearance(store, 3, blue);
        }
        15 => paint_appearance(store, 1, red),
        19 => {
            insert_row(store, TableId::Countries, 3, 85);
            write_cell(store, TableId::Countries, 3, "canonical_slot", Cell::U32(1));
        }
        _ => {}
    });
    certified("old colour source", units, content(), &[12, 24, 36], |hour, installer, _| {
        let truth = match hour {
            ..15 => shade(NATIONS[1].3),
            15..19 => shade(red),
            _ => shade(blue),
        };
        let colour = installer.colours.get(2).copied().flatten();
        (colour != truth).then(|| format!("key 2 is {colour:?}, not {truth:?}"))
    });
}

#[test]
fn a_recovery_whole_after_a_refused_write_carries_the_ends_borders() {
    let units = politics("political refused recovery", 1..=0, |at, store| {
        if at == 15 {
            hold(store, 2, 8, 8);
        }
    });
    let mut side = political_side(units, content());
    let mut sets = Vec::new();
    frame_to(&mut side, &mut sets, 12);
    let layer = political(&side);
    let mut installer = Installer::new(Arc::clone(layer.sets.as_ref().unwrap()), Arc::default(), Arc::clone(&layer.asks_whole), Arc::new(std::sync::atomic::AtomicBool::new(true)), Arc::clone(&layer.generation));
    let (mut screen, mut phases) = (Screen::default(), [0; 4]);
    for set in sets.drain(..) {
        installer.sets.push_back(set);
    }
    let picked = installer.pick(Some(12));
    installer.write(&mut screen, picked, Some(12), &mut phases).unwrap();
    assert!(screen.plans.is_some(), "the first whole fill carries its borders");
    let before = screen.plans.clone();

    frame_to(&mut side, &mut sets, 24);
    for set in sets.drain(..) {
        installer.sets.push_back(set);
    }
    screen.refuse = true;
    let picked = installer.pick(Some(24));
    assert!(picked.plans.is_some(), "the transfer at 15 moved a country border");
    installer.write(&mut screen, picked, Some(24), &mut phases).unwrap();
    assert!(installer.refused && political(&side).asks_whole.load(Ordering::Acquire), "the refused write asks the side for the whole fill");
    assert_eq!(screen.plans, before, "and the refused hour's borders are not written");

    screen.refuse = false;
    frame_to(&mut side, &mut sets, 36);
    for set in sets.drain(..) {
        installer.sets.push_back(set);
    }
    let picked = installer.pick(Some(36));
    assert!(picked.whole.is_some(), "the side answered with the whole fill");
    let written = screen.plans_written;
    installer.write(&mut screen, picked, Some(36), &mut phases).unwrap();
    let layer = political(&side);
    let end = layer.classes.clone().zip(layer.zone_plan.clone());
    assert!(screen.plans_written > written, "the recovery whole writes borders");
    assert_eq!(screen.plans, end, "and they are the borders the end leaves");
    assert_ne!(screen.plans, before, "which the transfer at 15 changed");
}
